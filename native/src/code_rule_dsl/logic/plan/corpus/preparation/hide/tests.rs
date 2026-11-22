    use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
    use crate::code_rule_dsl::logic::program_image::manager::Pipeline;
    use crate::code_rule_dsl::logic::program_image::{Builder, Version};
    use std::sync::Arc;

    /// `text` hidden with `bound`.
    fn hidden(text: &str, bound: u32) -> Arc<Version> {
        let mut p = Pipeline::new("test", crate::code_rule_dsl::logic::rr::parse_version(text).unwrap());
        p.program(&mut super::Hide::new(bound));
        p.finish().unwrap().0
    }

    /// The rules of `rel`, shown.
    fn rules_of(v: &Version, rel: &str) -> Vec<String> {
        v.rules().filter(|r| v.rel_name(r.head().rel()) == rel).map(|r| v.show_rule(r)).collect()
    }

    /// A kind set read negated over a guest's node (`kind(r, k), !ok_kind(k)`) reads its qualified copy: the bare set
    /// holds none of the guest's kinds. The port first left negated reads alone, and the Python pack's combine stopped
    /// on the debug check that the tree's pass and the version's agree.
    #[test]
    fn a_negated_kind_set_over_a_guest_reads_its_copy() {
        let v = hidden(
            ".decl parsed(s: node, g: symbol, r: node)\n.decl kind(n: node, k: symbol)\n.input parsed, kind\n\
             .decl ok_kind(k: symbol)\nok_kind(\"select\").\n\
             .decl out(r: node)\n.output out\nout(r) :- parsed(_, \"sql\", r), kind(r, k), !ok_kind(k).\n",
            1 << 30,
        );
        assert!(v.show().contains("!rr_in_sql_rows_ok_kind("), "{}", v.show());
    }

    /// A guest's kind tested against a host relation's rows, read negated, is tested against the rows' copy in the
    /// guest's grammar: the port copied only a positive read (`!def_or_class(k)` kept the host's rows, so no guest
    /// kind was one), and the combine's check stopped on the 317 rules of ep2.
    #[test]
    fn a_negated_read_of_a_kinds_rows_is_copied_for_the_guest() {
        let v = hidden(
            concat!(
                ".decl parsed(s: node, g: symbol, r: node)\n.input parsed\n",
                ".decl def_or_class(k: symbol)\ndef_or_class(\"class_definition\").\n",
                ".decl out(n: node)\n.output out\nout(r) :- parsed(_, \"python\", r), kind(r, k), !def_or_class(k).\n",
            ),
            1 << 30,
        );
        let out = rules_of(&v, "out");
        assert!(out.iter().any(|r| r.contains("!rr_in_python_rows_def_or_class(")), "{out:?}");
    }

    /// A relation read with guest variables in two columns its rules bind apart is copied once, with both seeded:
    /// copying the copy lost the first seed, so its reader read it at that column again, and schema-architecture's
    /// `rr_agg36` nested `rr_in_annotation_1_rr_in_annotation_0_…` without end.
    #[test]
    fn a_copy_read_at_another_guest_column_is_one_copy_of_the_relation() {
        let v = hidden(
            concat!(
                ".decl parsed(s: node, g: symbol, r: node)\n.decl e(a: node)\n.decl f(b: node)\n.input parsed, e, f\n",
                ".decl pair(a: node, b: node)\npair(a, b) :- e(a), f(b).\n",
                ".decl out(r: node)\n.output out\nout(r) :- parsed(_, \"sql\", r), parsed(_, \"sql\", s), pair(r, s).\n",
            ),
            1 << 30,
        );
        let copies: std::collections::BTreeSet<&str> = v.rules().map(|r| v.rel_name(r.head().rel())).filter(|r| r.starts_with("rr_in_")).collect();
        assert!(copies.iter().all(|c| c.matches("rr_in_").count() == 1), "{copies:?}");
        assert!(copies.contains("rr_in_sql_0_sql_1_pair"), "{copies:?}");
        let out = rules_of(&v, "out");
        assert!(out.iter().any(|r| r.contains("rr_in_sql_0_sql_1_pair(")), "{out:?}");
    }

    /// O110: a column filled by host and guest nodes alike (`calls.rerail`'s `tv`) is not a guest's column. A rule reading
    /// a relation with a variable from it keeps reading the relation for the host's nodes, and a clone reads the
    /// guest's copy; before, the rule read only the copy, and `tbv` lost every host assignment.
    #[test]
    fn a_maybe_guest_variable_keeps_the_host_rows_and_clones_for_the_guest() {
        let v = hidden(
            concat!(
                ".decl parsed(s: node, g: symbol, r: node)\n.decl e(a: node)\n.input parsed, e\n",
                ".decl val(n: node)\nval(n) :- e(n).\nval(r) :- parsed(_, \"sql\", r).\n",
                ".decl asg(n: node)\nasg(n) :- e(n).\n",
                ".decl only(n: node)\nonly(r) :- parsed(_, \"sql\", r).\n",
                ".decl out(n: node)\n.output out\nout(n) :- val(n), asg(n).\nout(n) :- only(n), asg(n).\n",
            ),
            1 << 30,
        );
        let outs = rules_of(&v, "out");
        assert!(outs.iter().any(|r| r.contains("val(") && r.contains(" asg(")), "the host's rows stay: {outs:#?}");
        assert!(outs.iter().any(|r| r.contains("val(") && r.contains("rr_in_sql_0_asg(")), "a clone reads the guest's copy: {outs:#?}");
        // A column only guest nodes fill is the guest's: its reader is specialized, not split.
        assert!(outs.iter().any(|r| r.contains("only(") && r.contains("rr_in_sql_0_asg(")), "{outs:#?}");
        assert!(!outs.iter().any(|r| r.contains("only(") && r.contains(" asg(")), "{outs:#?}");
    }

    /// A copy whose head holds a constant at the seeded column (`unset` is -1) is never carried as scoped there, so
    /// every round found the same copy again and counted it a change: `ruff-perf401` did not end (decision 0084's
    /// dump found it). Reading the copy already is no change, and the rounds end.
    #[test]
    fn a_copy_with_a_constant_at_its_seeded_column_ends() {
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let v = hidden(
                concat!(
                    ".decl parsed(s: node, g: symbol, r: node)\n.input parsed\n",
                    ".decl none(n: node)\nnone(-1) :- parsed(_, _, _).\n",
                    ".decl out(r: node)\n.output out\nout(r) :- parsed(_, \"sql\", r), none(r).\n",
                ),
                1 << 30,
            );
            let _ = done.send(rules_of(&v, "out"));
        });
        let out = wait.recv_timeout(std::time::Duration::from_secs(20)).expect("hide ends");
        assert!(out.iter().any(|r| r.contains("rr_in_sql_0_none(")), "{out:?}");
    }

    /// A copy for a guest in the run's own language (an annotation in a Python run, its guests shown) is its
    /// original: folded once the language is known (`FoldOwn`), its reader reads the original and its clone repeats
    /// its rule. In another language's run the copies stay.
    #[test]
    fn copies_for_a_guest_in_the_runs_own_language_are_folded_into_their_originals() {
        use crate::code_rule_dsl::facts::vtab::syntax::{RUN_VALUE_KEYS, HIDDEN_KEYS};
        let text = |g: &str| {
            format!(concat!(
                ".decl parsed(s: node, g: symbol, r: node)\n.decl e(a: node)\n.input parsed, e\n",
                ".decl val(n: node)\nval(n) :- e(n).\nval(r) :- parsed(_, \"{g}\", r).\n",
                ".decl asg(n: node)\nasg(n) :- e(n), kind(n, \"identifier\").\n",
                ".decl only(n: node)\nonly(r) :- parsed(_, \"{g}\", r).\n",
                ".decl out(n: node)\n.output out\nout(n) :- val(n), asg(n).\nout(n) :- only(n), asg(n).\n",
            ), g = g)
        };
        // Hidden, its own guests' copies folded, and merged, as the preparation runs them.
        let folded = |grammar: &str, label: &str| -> (usize, usize, Vec<String>) {
            let read = crate::code_rule_dsl::logic::rr::parse_version(&text(grammar)).unwrap();
            let mut p = Pipeline::new("test", read);
            p.program(&mut super::Hide::new(RUN_VALUE_KEYS));
            let (v, _) = p.finish().unwrap();
            let made = v.declared_names().iter().filter(|r| r.starts_with("rr_in_")).count();
            let mut p = Pipeline::new("test", v);
            let mut fold = super::super::combining::FoldOwn { label, folded: 0, diffed: Default::default() };
            let (mut merge, mut repeats) = (super::super::merge::MergeIdentical::default(), super::super::merge::DropRepeated::default());
            p.program(&mut fold).program(&mut repeats).program(&mut merge).program(&mut repeats);
            let (v, _) = p.finish().unwrap();
            (made, fold.folded, v.rules().map(|r| v.show_rule(r)).collect())
        };
        let (made, n, rules) = folded("python", "python");
        assert!(made > 0, "hiding made the guest's copies");
        assert_eq!(n, made);
        assert!(rules.iter().all(|r| !r.contains("rr_in_") && !r.contains("python:")), "{rules:#?}");
        let outs: Vec<&String> = rules.iter().filter(|r| r.starts_with("out(")).collect();
        assert_eq!(outs.len(), 2, "each clone repeats its rule: {outs:#?}");
        assert_eq!(folded("python", "rust").1, 0, "another language's run keeps them");
        // An annotation is not shown to a rule that does not name it, so its copies fold into originals whose
        // bounds take annotations in; a rule no annotation reaches keeps the narrow bound.
        let (made, n, rules) = folded("annotation", "python");
        assert!(made > 0 && n == made, "{made} made, {n} folded");
        let wide = (i64::from(HIDDEN_KEYS) << 32).to_string();
        let narrow = (i64::from(RUN_VALUE_KEYS) << 32).to_string();
        assert!(rules.iter().any(|r| r.starts_with("asg(") && r.contains(&wide)), "{rules:#?}");
        assert!(rules.iter().any(|r| r.starts_with("out(") && r.contains(&narrow)) || rules.iter().filter(|r| r.starts_with("out(")).all(|r| !r.contains(&wide)), "{rules:#?}");
    }

    /// An original widened for an annotation's copies is read as hiding left it by the rules that are not its own and
    /// reach no annotation (decision 0165 §3, main's #923): `rr_narrow_<orig>`, its rules under the narrow bound. A
    /// native operator whose relation reads narrow reads the narrow copy too: its rows are made from the relation its
    /// fields name, not from its rule.
    #[test]
    fn a_rule_reaching_no_annotation_reads_the_widened_original_narrow() {
        use crate::code_rule_dsl::facts::vtab::syntax::{HIDDEN_KEYS, RUN_VALUE_KEYS};
        use crate::code_rule_dsl::logic::front::Native;
        use crate::code_rule_dsl::logic::program_facts::Field;
        use crate::code_rule_dsl::logic::program_image::fields::Edit;
        let v = hidden(
            concat!(
                ".decl parsed(s: node, g: symbol, r: node)\n.decl e(a: node)\n.input parsed, e\n",
                ".decl asg(n: node)\nasg(n) :- e(n), kind(n, \"identifier\").\n",
                ".decl only(n: node)\nonly(r) :- parsed(_, \"annotation\", r).\n",
                ".decl named(n: node)\nnamed(n) :- asg(n).\n",
                ".decl out(n: node)\n.output out\nout(n) :- only(n), asg(n).\nout(n) :- named(n).\n",
            ),
            RUN_VALUE_KEYS,
        );
        let v = {
            let mut e = Edit::on(&v);
            let mut natives = v.native_map().unwrap();
            natives.insert("named".into(), Native::TextTest { input: "asg".into(), column: 0, op: "match".into(), patterns: Vec::new() });
            e.b.fields().set_field(Field::Natives(&natives));
            e.finish().unwrap()
        };
        let mut p = Pipeline::new("test", v);
        let mut fold = super::super::combining::FoldOwn { label: "python", folded: 0, diffed: Default::default() };
        p.program(&mut fold);
        let (v, _) = p.finish().unwrap();
        assert!(fold.folded > 0);
        let wide = (i64::from(HIDDEN_KEYS) << 32).to_string();
        let asg = rules_of(&v, "asg");
        assert!(!asg.is_empty() && asg.iter().all(|r| r.contains(&wide)), "the original takes the annotations in: {asg:#?}");
        let narrow = rules_of(&v, "rr_narrow_asg");
        assert!(!narrow.is_empty() && narrow.iter().all(|r| !r.contains(&wide)), "its narrow copy does not: {narrow:#?}");
        let named = rules_of(&v, "named");
        assert!(!named.is_empty() && named.iter().all(|r| r.contains("rr_narrow_asg(")), "{named:#?}");
        assert_eq!(v.native_map().unwrap()["named"].reads(), ["rr_narrow_asg"]);
        assert!(v.declares("rr_narrow_asg"));
    }

    /// A widened original whose every row is of one document, made from that document's rows alone, holds as hiding
    /// left it the rows of the documents under the narrow bound: its narrow copy is those rows of it, not its rules
    /// again (corpus0, one pack, 2026-11-21: 586 narrow copies, 15 s, beside their originals). One whose rows join
    /// another widened original by a value (`pair(n)`, an `asg` of any document with `n`'s text) is made again under
    /// the narrow bound, as #923 made it: widened, it would read the annotations' `asg`.
    #[test]
    fn a_narrow_copy_of_an_original_within_one_document_is_its_rows_under_the_narrow_bound() {
        use crate::code_rule_dsl::facts::vtab::syntax::{HIDDEN_KEYS, RUN_VALUE_KEYS};
        let v = hidden(
            concat!(
                ".decl parsed(s: node, g: symbol, r: node)\n.decl e(a: node)\n.input parsed, e\n",
                ".decl asg(n: node)\nasg(n) :- e(n), kind(n, \"identifier\").\n",
                ".decl pair(n: node)\npair(n) :- e(n), text(n, t), asg(m), text(m, t).\n",
                ".decl only(n: node)\nonly(r) :- parsed(_, \"annotation\", r).\n",
                ".decl named(n: node)\nnamed(n) :- asg(n).\n",
                ".decl paired(n: node)\npaired(n) :- pair(n).\n",
                ".decl out(n: node)\n.output out\n",
                "out(n) :- only(n), asg(n).\nout(n) :- only(n), pair(n).\nout(n) :- named(n).\nout(n) :- paired(n).\n",
            ),
            RUN_VALUE_KEYS,
        );
        let mut p = Pipeline::new("test", v);
        let mut fold = super::super::combining::FoldOwn { label: "python", folded: 0, diffed: Default::default() };
        p.program(&mut fold);
        let (v, _) = p.finish().unwrap();
        assert!(fold.folded > 0);
        let all: Vec<String> = v.rules().map(|r| v.show_rule(r)).collect();
        let (wide, narrow) = ((i64::from(HIDDEN_KEYS) << 32).to_string(), (i64::from(RUN_VALUE_KEYS) << 32).to_string());
        let asg = rules_of(&v, "asg");
        assert!(!asg.is_empty() && asg.iter().all(|r| r.contains(&wide)), "the original takes the annotations in: {all:#?}");
        let copy = rules_of(&v, "rr_narrow_asg");
        assert_eq!(copy.len(), 1, "{all:#?}");
        assert!(copy[0].contains(" asg(") && copy[0].contains(&narrow) && !copy[0].contains("e(") && !copy[0].contains(&wide), "the original's rows under the narrow bound: {all:#?}");
        assert!(rules_of(&v, "named").iter().all(|r| r.contains("rr_narrow_asg(")), "{all:#?}");
        let pair = rules_of(&v, "rr_narrow_pair");
        assert!(!pair.is_empty() && pair.iter().all(|r| r.contains("rr_narrow_asg(") && !r.contains(" pair(") && !r.contains(&wide)), "made again, narrow: {all:#?}");
        assert!(rules_of(&v, "paired").iter().all(|r| r.contains("rr_narrow_pair(")), "{all:#?}");
    }

    /// A narrow copy is its original's rows under the narrow bound where its head holds a constant for no node (`-1`,
    /// `param_parts`' shape on corpus0; `whole` reads it, its `-1` or its node), and where it joins its document through
    /// a relation's number column that its
    /// rules make a document's (`modnode(d: number, m: node) :- node(d, m, …)`, `modname`'s shape): neither is
    /// another document.
    #[test]
    fn a_narrow_copy_is_its_originals_rows_under_the_narrow_bound_through_constants_and_document_columns() {
        use crate::code_rule_dsl::facts::vtab::syntax::{HIDDEN_KEYS, RUN_VALUE_KEYS};
        let v = hidden(
            concat!(
                ".decl parsed(s: node, g: symbol, r: node)\n.decl e(a: node)\n.input parsed, e\n",
                ".decl part(n: node, d: node)\npart(n, -1) :- e(n), kind(n, \"identifier\").\npart(n, d) :- e(n), field(n, \"value\", d).\n",
                ".decl modnode(d: number, m: node)\nmodnode(d, m) :- node(d, m, \"module\", _, _).\n",
                ".decl modname(m: node, q: symbol)\nmodname(m, q) :- modnode(d, m), doc(d, _, q).\n",
                ".decl only(n: node)\nonly(r) :- parsed(_, \"annotation\", r).\n",
                ".decl pp(n: node)\npp(n) :- part(n, _).\n",
                ".decl whole(n: node, d: node)\nwhole(n, d) :- part(n, d), e(n).\n.decl wh(n: node)\nwh(n) :- whole(n, _).\n",
                ".decl sk(kind: symbol)\nsk(\"module\").\n.decl ee(n: node, s: node)\nee(n, s) :- nearest(n, s, sk).\n.decl ex(n: node)\nex(n) :- ee(n, _).\n",
                ".decl mn(n: node)\nmn(n) :- modname(n, _).\n",
                ".decl out(n: node)\n.output out\n",
                "out(n) :- only(n), part(n, _).\nout(n) :- only(n), modname(n, _).\nout(n) :- only(n), whole(n, _).\nout(n) :- only(n), ee(n, _).\nout(n) :- pp(n).\nout(n) :- mn(n).\nout(n) :- wh(n).\nout(n) :- ex(n).\n",
            ),
            RUN_VALUE_KEYS,
        );
        let mut p = Pipeline::new("test", v);
        let mut fold = super::super::combining::FoldOwn { label: "python", folded: 0, diffed: Default::default() };
        p.program(&mut fold);
        let (v, _) = p.finish().unwrap();
        let all: Vec<String> = v.rules().map(|r| v.show_rule(r)).collect();
        let (wide, narrow) = ((i64::from(HIDDEN_KEYS) << 32).to_string(), (i64::from(RUN_VALUE_KEYS) << 32).to_string());
        for rel in ["part", "modname", "whole"] {
            let copy = rules_of(&v, &format!("rr_narrow_{rel}"));
            assert!(copy.len() == 1 && copy[0].contains(&format!(" {rel}(")) && copy[0].contains(&narrow) && !copy[0].contains(&wide), "{rel}, its rows under the narrow bound: {all:#?}");
        }
        // `ee` reads no bound (`scope_node`'s shape on corpus0): widened or not, its rows are the same, and its readers
        // read it.
        assert!(rules_of(&v, "ee").iter().all(|r| !r.contains(&narrow) && !r.contains(&wide)), "{all:#?}");
        assert!(rules_of(&v, "rr_narrow_ee").is_empty() && rules_of(&v, "ex").iter().all(|r| r.contains(" ee(")), "ee is read as it is: {all:#?}");
    }

    /// A narrow copy that holds an annotation's rows is made again, not its original's rows under the narrow bound:
    /// `wstart` on corpus0 joins an annotation's name (`n`, from `parsed_in`'s tree) to its host file's scopes (`s`)
    /// through the annotation's copy of `nameref`, which hiding wrote and its narrow copy reads widened too; `walk` and
    /// `hosts` read it, and `parsed_in` holds an annotation's nodes itself. `nameref` and `astr`, bounded in one
    /// document, are their originals' rows under the bound.
    #[test]
    fn a_narrow_copy_that_holds_an_annotations_rows_is_made_again() {
        use crate::code_rule_dsl::facts::vtab::syntax::RUN_VALUE_KEYS;
        let v = hidden(
            concat!(
                ".decl parsed(s: node, g: symbol, r: node)\n.decl e(a: node)\n.input parsed, e\n",
                ".decl nameref(n: node)\nnameref(n) :- e(n), kind(n, \"identifier\").\n",
                ".decl astr(s: node)\nastr(s) :- e(s), kind(s, \"string\").\n",
                ".decl parsed_in(x: node, s: node)\nparsed_in(x, s) :- astr(s), parsed(s, \"annotation\", x).\n",
                ".decl wstart(n: node, s: node)\nwstart(n, s) :- nameref(n), parsed_in(n, h), child(s, h, _).\n",
                ".decl walk(n: node, s: node)\nwalk(n, s) :- wstart(n, s).\nwalk(n, p) :- walk(n, s), child(p, s, _).\n",
                ".decl hosts(s: node)\nhosts(s) :- wstart(_, s).\n",
                ".decl out(n: node)\n.output out\nout(n) :- walk(n, _).\nout(n) :- hosts(n).\nout(n) :- parsed_in(n, _).\n",
            ),
            RUN_VALUE_KEYS,
        );
        let read: std::collections::BTreeSet<String> = ["nameref", "astr", "parsed_in", "wstart", "walk", "hosts"].map(str::to_owned).into();
        let folded: std::collections::HashMap<String, String> = v.rules().map(|r| v.rel_name(r.head().rel()).to_owned()).filter(|h| h.starts_with("rr_in_annotation_")).map(|h| (h.clone(), h.trim_start_matches(|c: char| c != '_').trim_start_matches("_in_annotation_").trim_start_matches(|c: char| c.is_ascii_digit()).trim_start_matches('_').to_owned())).collect();
        assert!(folded.get("rr_in_annotation_0_nameref").is_some_and(|o| o == "nameref"), "{folded:?}");
        assert!(v.rules().map(|r| v.show_rule(r)).any(|r| r.starts_with("wstart(") && r.contains("rr_in_annotation_0_nameref(")), "wstart reads the annotation's copy");
        let (within, same) = super::super::combining::filters_of(&v, &read, &folded);
        assert!(same.is_empty(), "each reads one that differs: {same:?}");
        let all: Vec<String> = v.rules().map(|r| v.show_rule(r)).collect();
        for rel in ["nameref", "astr"] {
            assert!(within.contains(rel), "{rel} is its original's rows under the bound: {within:?} {all:#?}");
        }
        for rel in ["parsed_in", "wstart", "walk", "hosts"] {
            assert!(!within.contains(rel), "{rel} is made again: {within:?} {all:#?}");
        }
    }

    /// A tree axis read over a node that may be an annotation's is the clone's copy of the axis, over a copy of its kind
    /// set (`rr_inside_rr_in_annotation_<set>`). Folding the copied set folds the axis too: before, the set went and
    /// the axis stayed, naming no set, so the transducer made it with no rows, and the clone's `!inside(t, _, S)` held
    /// for every node, below S too (the Django rules' `within_stop` over a block found through `sym_of`; main's #862).
    /// The output shows `t`'s text, not `t`: a clone whose head would show an annotation's node is dropped (decision
    /// 0165), and one that reads it inside its body stays, as here.
    #[test]
    fn an_axis_over_a_folded_kind_set_reads_the_original_set() {
        use crate::code_rule_dsl::facts::vtab::syntax::RUN_VALUE_KEYS;
        use crate::code_rule_dsl::logic::front::{axis, is_axis};
        let v = hidden(
            concat!(
                ".decl parsed(s: node, g: symbol, r: node)\n.decl e(a: node)\n.input parsed, e\n",
                ".decl body(b: node)\nbody(b) :- e(b).\nbody(r) :- parsed(_, \"annotation\", r).\n",
                ".decl attempt(t: node)\nattempt(t) :- e(t).\n",
                ".decl out(x: symbol)\n.output out\n",
                "out(x) :- attempt(t), body(b), within(t, b), !inside(t, _, \"function_definition\"), text(t, x).\n",
            ),
            RUN_VALUE_KEYS,
        );
        let axes = |v: &Version| -> Vec<String> { v.rules().flat_map(|r| v.show_rule(r).split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| is_axis(w)).map(str::to_owned).collect::<Vec<_>>()).collect() };
        assert!(axes(&v).iter().any(|r| r.contains("rr_in_annotation_")), "the clone reads the annotation's axis: {:?}", axes(&v));
        let mut p = Pipeline::new("test", v);
        let mut fold = super::super::combining::FoldOwn { label: "python", folded: 0, diffed: Default::default() };
        let (mut merge, mut repeats) = (super::super::merge::MergeIdentical::default(), super::super::merge::DropRepeated::default());
        p.program(&mut fold).program(&mut repeats).program(&mut merge).program(&mut repeats);
        let (v, _) = p.finish().unwrap();
        assert!(fold.folded > 0);
        let sets = ProgramFields::kindsets(&*v);
        let inputs: Vec<String> = v.inputs().map(|a| v.text(a).to_owned()).filter(|r| is_axis(r)).collect();
        for rel in axes(&v).iter().chain(&inputs) {
            let (set, _) = axis(rel).unwrap();
            assert!(sets.contains_key(set), "{rel} names no kind set of the program: {sets:?}");
        }
        let outs = rules_of(&v, "out");
        assert!(!outs.is_empty() && outs.iter().all(|r| r.contains("!rr_inside_rr_kind_function_definition(") && !r.contains("rr_in_")), "{outs:#?}");
    }

    /// 0597 rung 2.4: what a context holds is its own pass's, never widened, copied or narrowed for the host's
    /// annotations. Hidden at the run values' bound and folded as a pack folds it, the context's instances
    /// (`rr_cx_quoted_*`) get no `rr_in_annotation_` copy and no `rr_narrow_` copy, and the host's `ident`, read by both
    /// passes as written, stays the host's: no rule of it takes the annotations in.
    #[test]
    fn what_a_context_holds_is_never_copied_widened_or_narrowed() {
        use crate::code_rule_dsl::facts::vtab::syntax::{HIDDEN_KEYS, RUN_VALUE_KEYS};
        let folded = |v: Arc<Version>| {
            let mut p = Pipeline::new("test", v);
            let mut fold = super::super::combining::FoldOwn { label: "python", folded: 0, diffed: Default::default() };
            p.program(&mut fold);
            p.finish().unwrap().0
        };
        // The control: the same reach written as the library writes it today, into the annotation's trees from the
        // host's rules, is copied for the annotation and its original widened.
        let today = folded(hidden(
            concat!(
                ".language python\n",
                ".decl parsed(s: node, g: symbol, r: node)\n.decl string_annotation(s: node)\n.decl kind(n: node, k: symbol)\n.decl text(n: node, s: symbol)\n",
                ".decl within(n: node, a: node)\n.input parsed, string_annotation, kind, text, within\n",
                ".decl ident(i: node)\nident(i) :- kind(i, \"identifier\").\n",
                ".decl host_names(x: symbol)\nhost_names(x) :- ident(i), text(i, x).\n",
                ".decl quoted(x: symbol)\nquoted(x) :- string_annotation(s), parsed(s, \"annotation\", r), within(i, r), ident(i), text(i, x).\n",
                ".output host_names, quoted\n",
            ),
            RUN_VALUE_KEYS,
        ));
        let wide = (i64::from(HIDDEN_KEYS) << 32).to_string();
        let shown: Vec<String> = today.rules().map(|r| today.show_rule(r)).collect();
        assert!(shown.iter().any(|r| r.contains("rr_in_annotation") || r.contains("rr_narrow_")) || rules_of(&today, "ident").iter().any(|r| r.contains(&wide)), "the control copies or widens: {shown:#?}");
        let v = hidden(
            concat!(
                ".language python\n",
                ".decl parsed(s: node, g: symbol, r: node)\n.decl string_annotation(s: node)\n.decl kind(n: node, k: symbol)\n.decl text(n: node, s: symbol)\n",
                ".decl within(n: node, a: node)\n.input parsed, string_annotation, kind, text, within\n",
                ".decl ident(i: node)\nident(i) :- kind(i, \"identifier\").\n",
                ".decl host_names(x: symbol)\nhost_names(x) :- ident(i), text(i, x).\n",
                ".decl quoted_root(r: node)\nquoted_root(r) :- string_annotation(s), parsed(s, \"annotation\", r).\n",
                ".decl ann_ident(x: symbol)\nann_ident(x) :- ident(i), text(i, x).\n",
                ".decl quoted(x: symbol)\nquoted(x) :- names(x).\n",
                ".output host_names, quoted\n",
                ".context quoted of python roots quoted_root exports ann_ident\n",
                ".from quoted ann_ident as names\n",
            ),
            RUN_VALUE_KEYS,
        );
        let v = folded(v);
        let all: Vec<String> = v.rules().map(|r| v.show_rule(r)).collect();
        let names: Vec<String> = v.declared_names().iter().map(|n| n.to_string()).chain(v.rules().map(|r| v.rel_name(r.head().rel()).to_owned())).collect();
        assert!(names.iter().all(|n| !n.starts_with("rr_in_annotation") && !n.starts_with("rr_narrow_")), "no copy: {all:#?}");
        assert!(all.iter().any(|r| r.starts_with("rr_cx_quoted_ann_ident(")), "the context's own: {all:#?}");
        assert!(rules_of(&v, "ident").iter().all(|r| !r.contains(&wide)), "the host's ident takes no annotation in: {all:#?}");
    }

    /// A widened original whose every row is one document's (`Local`) holds, under the narrow bound, the rows its
    /// narrow copy would: a reader that bounds its node reads the original, and no copy is made. Made apart, the copies
    /// derived hogql's `nameref`, `attr_of` and `walk` a second time (main's #923: 90.5 B instructions to 102.9 B).
    #[test]
    fn a_local_original_read_bounded_is_read_in_place_not_copied() {
        use crate::code_rule_dsl::facts::vtab::syntax::{HIDDEN_KEYS, RUN_VALUE_KEYS};
        // `named` bounds its node narrow, as hiding bounds a rule no annotation reaches.
        let bound = i64::from(RUN_VALUE_KEYS) << 32;
        let v = hidden(
            &format!(
                "{}{}{}{}{}",
                ".decl parsed(s: node, g: symbol, r: node)\n.decl e(a: node)\n.input parsed, e\n",
                ".decl asg(n: node)\nasg(n) :- e(n), kind(n, \"identifier\").\n",
                ".decl only(n: node)\nonly(r) :- parsed(_, \"annotation\", r).\n",
                format_args!(".decl named(n: node)\nnamed(n) :- asg(n), n < {bound}.\n"),
                ".decl out(n: node)\n.output out\nout(n) :- only(n), asg(n).\nout(n) :- named(n).\n",
            ),
            RUN_VALUE_KEYS,
        );
        let mut p = Pipeline::new("test", v);
        let mut fold = super::super::combining::FoldOwn { label: "python", folded: 0, diffed: Default::default() };
        p.program(&mut fold);
        let (v, _) = p.finish().unwrap();
        assert!(fold.folded > 0);
        let (wide, narrow) = ((i64::from(HIDDEN_KEYS) << 32).to_string(), (i64::from(RUN_VALUE_KEYS) << 32).to_string());
        let asg = rules_of(&v, "asg");
        assert!(!asg.is_empty() && asg.iter().all(|r| r.contains(&wide)), "the original takes the annotations in: {asg:#?}");
        assert!(rules_of(&v, "rr_narrow_asg").is_empty() && !v.declares("rr_narrow_asg"), "{:#?}", v.rules().map(|r| v.show_rule(r)).collect::<Vec<_>>());
        let named = rules_of(&v, "named");
        assert!(!named.is_empty() && named.iter().all(|r| r.contains("asg(") && r.contains(&narrow) && !r.contains(&wide)), "read bounded: {named:#?}");
    }
