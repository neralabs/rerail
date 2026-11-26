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

    /// A guest node's kind compared with constants (`kind(p, k), (k = "identifier" ; …)`, tsx/names' `bound_name`) is
    /// compared with the guest grammar's kinds, the node's grammar known as a context's are (decision 0597): left
    /// unqualified, each is the run language's kind, or none (a Vue run's -1), and a v-for alias bound nothing.
    #[test]
    fn a_guest_kind_compared_with_constants_is_the_guests() {
        let v = hidden(
            concat!(
                ".decl parsed(s: node, g: symbol, r: node)\n.input parsed\n",
                ".decl out(x: symbol)\n.output out\n",
                "out(x) :- parsed(_, \"typescript\", g), within(p, g), kind(p, k), (k = \"identifier\" ; k = \"shorthand_property_identifier_pattern\"), text(p, x).\n",
            ),
            1 << 30,
        );
        let out = rules_of(&v, "out");
        assert!(out.iter().all(|r| r.contains("\"typescript:")), "{out:?}");
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

    /// A kind compared over a Blade template's compiled view (`compiled-view`, built at the file's load) is named in
    /// PHP, the one dialect the embedding gives (`kinds_label`). The embedding's own name is no dialect's label: a kind
    /// qualified by it was not `qualified`, so each round qualified it again (`compiled-view:compiled-view:comment`)
    /// and hiding did not end (phpdoc's `doc_code`, `k != "comment"`, in bladestan-if-always-true).
    #[test]
    fn a_kind_compared_over_a_blade_view_is_phps_and_ends() {
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let v = hidden(
                concat!(
                    ".decl parsed(s: node, g: symbol, r: node)\n.input parsed\n",
                    ".decl out(p: node)\n.output out\n",
                    "out(p) :- parsed(_, \"compiled-view\", g), within(p, g), kind(p, k), k != \"comment\".\n",
                ),
                1 << 30,
            );
            let _ = done.send(rules_of(&v, "out"));
        });
        let out = wait.recv_timeout(std::time::Duration::from_secs(20)).expect("hide ends");
        assert!(out.iter().all(|r| r.contains("\"php:comment\"")), "{out:?}");
    }

    /// A tree axis read over a node that may be a guest's is the clone's copy of the axis, over a copy of its kind set
    /// (`rr_inside_rr_in_annotation_<set>`), and the set is the program's: the clone's `!inside(t, _, S)` reads S's
    /// copy, never an axis naming no set (main's #862, where the set went and the axis stayed). The output shows `t`'s
    /// text, not `t`: a clone whose head would show an annotation's node is dropped (decision 0165).
    #[test]
    fn an_axis_over_a_guests_kind_set_names_a_set_of_the_program() {
        use crate::code_rule_dsl::logic::front::{axis, is_axis};
        let v = hidden(
            concat!(
                ".decl parsed(s: node, g: symbol, r: node)\n.decl e(a: node)\n.input parsed, e\n",
                ".decl body(b: node)\nbody(b) :- e(b).\nbody(r) :- parsed(_, \"annotation\", r).\n",
                ".decl attempt(t: node)\nattempt(t) :- e(t).\n",
                ".decl out(x: symbol)\n.output out\n",
                "out(x) :- attempt(t), body(b), within(t, b), !inside(t, _, \"function_definition\"), text(t, x).\n",
            ),
            crate::code_rule_dsl::logic::plan::corpus::hiding::BOUND,
        );
        let axes: Vec<String> = v.rules().flat_map(|r| v.show_rule(r).split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| is_axis(w)).map(str::to_owned).collect::<Vec<_>>()).collect();
        assert!(axes.iter().any(|r| r.contains("rr_in_python_")), "the clone reads the axis in the annotation's kinds (Python's): {axes:?}");
        let sets = ProgramFields::kindsets(&*v);
        let inputs: Vec<String> = v.inputs().map(|a| v.text(a).to_owned()).filter(|r| is_axis(r)).collect();
        for rel in axes.iter().chain(&inputs) {
            let (set, _) = axis(rel).unwrap();
            assert!(sets.contains_key(set), "{rel} names no kind set of the program: {sets:?}");
        }
    }

    /// 0597 rung 2.4: what a context holds is its own pass's, never widened, copied or narrowed for the host's
    /// annotations. Hidden at the shown bound, the context's instances (`rr_pass_quoted_*`) get no `rr_in_annotation_`
    /// copy, and the host's `ident`, read by both passes as written, stays the host's: no rule of it takes the
    /// annotations in. Step 4 removed the fold and the narrow copies; neither name is made.
    #[test]
    fn what_a_context_holds_is_never_copied_widened_or_narrowed() {
        use crate::code_rule_dsl::logic::plan::corpus::hiding::BOUND;
        // The control: the same reach written into the annotation's trees from the host's rules is copied for the
        // annotation.
        let today = hidden(
            concat!(
                ".language python\n",
                ".decl parsed(s: node, g: symbol, r: node)\n.decl string_annotation(s: node)\n.decl kind(n: node, k: symbol)\n.decl text(n: node, s: symbol)\n",
                ".decl within(n: node, a: node)\n.input parsed, string_annotation, kind, text, within\n",
                ".decl ident(i: node)\nident(i) :- kind(i, \"identifier\").\n",
                ".decl host_names(x: symbol)\nhost_names(x) :- ident(i), text(i, x).\n",
                ".decl quoted(x: symbol)\nquoted(x) :- string_annotation(s), parsed(s, \"annotation\", r), within(i, r), ident(i), text(i, x).\n",
                ".output host_names, quoted\n",
            ),
            BOUND,
        );
        let shown: Vec<String> = today.rules().map(|r| today.show_rule(r)).collect();
        assert!(shown.iter().any(|r| r.contains("rr_in_annotation")), "the control copies: {shown:#?}");
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
            BOUND,
        );
        let all: Vec<String> = v.rules().map(|r| v.show_rule(r)).collect();
        let names: Vec<String> = v.declared_names().iter().map(|n| n.to_string()).chain(v.rules().map(|r| v.rel_name(r.head().rel()).to_owned())).collect();
        assert!(names.iter().all(|n| !n.starts_with("rr_in_annotation") && !n.starts_with("rr_narrow_")), "no copy: {all:#?}");
        assert!(all.iter().any(|r| r.starts_with("rr_pass_quoted_ann_ident(")), "the context's own: {all:#?}");
        assert!(rules_of(&v, "ident").len() == 1, "the host's ident is its rule as written, no clone: {all:#?}");
    }
