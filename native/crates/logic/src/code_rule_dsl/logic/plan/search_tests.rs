    use rerail_blocks::vector::Vector;
    use super::*;
    /// `cyclic_relations` by one pass over the edges is the walk from each relation: every relation that reaches
    /// itself through the given rules' atoms (a self-read, or a strongly connected part of more than one), over
    /// generated programs.
    #[test]
    fn cyclic_relations_are_those_that_reach_themselves() {
        let walked = |scc: &crate::code_rule_dsl::logic::front::Comp, v: &Version| -> BTreeSet<String> {
            let mut next: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
            for r in v.rules() {
                for l in r.body() {
                    if let super::super::program_image::Lit::Atom(a) = l.view()
                        && scc.contains(v.rel_name(a.rel()))
                    {
                        next.entry(v.rel_name(a.rel())).or_default().insert(v.rel_name(r.head().rel()));
                    }
                }
            }
            scc.iter().filter(|start| {
                let (mut seen, mut todo): (BTreeSet<&str>, Vec<&str>) = (BTreeSet::new(), next.get(start).into_iter().flatten().copied().collect());
                while let Some(x) = todo.pop() {
                    if x == *start { return true; }
                    if seen.insert(x) { todo.extend(next.get(x).into_iter().flatten().copied()); }
                }
                false
            }).map(str::to_owned).collect()
        };
        let mut seed = 7u64;
        let mut rand = |n: u64| { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; seed % n };
        for _ in 0..60 {
            let n = 2 + rand(12) as usize;
            let mut text: String = (0..n).map(|i| format!(".decl r{i}(x:number)\n")).collect();
            text.push_str(".decl base(x:number)\nbase(1).\n");
            for i in 0..n {
                text.push_str(&format!("r{i}(x) :- base(x).\n"));
                for _ in 0..rand(3) {
                    let j = rand(n as u64);
                    let neg = if rand(5) == 0 { format!(", !r{}(x)", rand(n as u64)) } else { String::new() };
                    text.push_str(&format!("r{i}(x) :- r{j}(x), base(x){neg}.\n"));
                }
            }
            text.push_str(&format!(".output {}\n", (0..n).map(|i| format!("r{i}")).collect::<Vec<_>>().join(",")));
            let p = super::super::rr::parse_version(&text).unwrap();
            // The relations taken as one part (a superset of each strongly connected part), with base outside it.
            let names: Vec<String> = (0..n).map(|i| format!("r{i}")).collect();
            let scc = &crate::code_rule_dsl::logic::front::Components::of_names([names.iter().map(String::as_str)])[0];
            let vs = body::Versions::of_placed(body::Placed::whole(p.clone()));
            let keys: Vec<RuleKey> = body::Versions::keys(0, p.rules().count()).collect();
            assert_eq!(cyclic_relations(&vs, scc, &keys), walked(scc, &p), "{text}");
        }
    }

    /// The guard as decision 0137 wrote it, over the same masks: a step sharing no variable with what is joined is
    /// refused while a connected step remains, where it or that connected step has no size of its own.
    fn cross_by_definition(i: usize, prev: u64, bound: u128, jmask: &[u128], no_size: &[bool]) -> bool {
        prev != 0 && jmask[i] & bound == 0 && (0..jmask.len()).any(|j| j != i && prev & (1 << j) == 0 && jmask[j] & bound != 0 && (no_size[i] || no_size[j]))
    }

    /// `unfounded_cross` (the DP's guard against a cross product priced from a default size) gives the definition's
    /// answer for every step, subset joined and set of bound variables, from masks made once per search: it read
    /// three statistics maps by relation name for every pair of joins at every candidate, 12% of `keeps::choose`.
    #[test]
    fn the_cross_product_guard_is_its_definition_over_masks() {
        // Four joins over variables 0..5: a(0,1) b(1,2) c(3) d(2,3,4).
        let jmask: Vec<u128> = vec![0b11, 0b110, 0b1000, 0b11100];
        for sized in 0u64..16 {
            let no_size: Vec<bool> = (0..4).map(|j| sized >> j & 1 == 0).collect();
            let mask: u64 = (0..4).filter(|&j| no_size[j]).map(|j| 1u64 << j).sum();
            for prev in 0u64..16 {
                let bound = (0..4).filter(|&j| prev >> j & 1 == 1).fold(0u128, |b, j| b | jmask[j]);
                for i in (0..4).filter(|&i| prev >> i & 1 == 0) {
                    assert_eq!(unfounded_cross(i, prev, bound, &jmask, mask), cross_by_definition(i, prev, bound, &jmask, &no_size), "step {i} after {prev:04b}, unsized joins {mask:04b}");
                }
            }
        }
    }

    /// Whether a filter is ready, kept per filter by which of its own variables are bound (`Readiness`): the answer
    /// is the test's for every set bound, and the test runs once per pattern of its variables. Kept in a hash map by
    /// (filter, variables), its lookups were two thirds of the time a search spent placing filters.
    #[test]
    fn a_filters_readiness_is_kept_by_its_own_variables() {
        // Three filters over variables 0..8: {1, 3}, {0, 2, 5, 7}, none.
        let own: Vec<u128> = vec![0b1010, 0b1010_0101, 0];
        let test = |f: usize, b: u128| match f {
            0 => b & 0b10 != 0,
            1 => b & 0b1000_0001 == 0b1000_0001,
            _ => true,
        };
        let mut runs = 0usize;
        let mut kept = Readiness::new(&own);
        for b in 0u128..256 {
            for (f, &mine) in own.iter().enumerate() {
                let got = kept.ready(f, b, |x| {
                    runs += 1;
                    assert_eq!(x & !mine, 0, "only the filter's own variables are passed");
                    test(f, x)
                });
                assert_eq!(got, test(f, b & mine), "filter {f} with {b:08b} bound");
            }
        }
        assert_eq!(runs, 4 + 16 + 1, "once per pattern of each filter's variables");
    }

    /// A model of `prog` whose inputs have the rows and distinct values by column given.
    fn model_with(prog: &body::Placed, inputs: &[(&str, f64, &[f64])]) -> Model {
        let given = Inputs { facts: "main.".to_owned(), stats: "main.".to_owned(), atoms: Some([0; 3]), native: BTreeSet::new(), batch: HashMap::new(), given: None, kept: BTreeSet::new() };
        let v = &prog.v;
        let derived: HashSet<String> = v.rules().map(|r| v.rel_name(r.head().rel()).to_owned()).collect();
        let mut model = Model { shapes: Default::default(), docs_by_shift: false, analysis: Default::default(), stats: Stats::empty_of(&prog.fields().unwrap()), ndv: Default::default(), docof: Default::default(), view_src: Default::default(), inputs: given.into(), derived: derived.iter().collect() };
        model.stats.nodes = 1_000_000.0;
        for &(rel, rows, ndv) in inputs {
            model.stats.card.insert(rel, rows);
            for (c, &n) in ndv.iter().enumerate() {
                model.ndv.insert((rel.to_owned(), c), n);
                model.stats.fan.insert((rel.to_owned(), c), rows / n);
            }
        }
        model
    }

    /// The rows after each step of a plan, as calibration reads them (`keeps::calibrated_rows_at`), are the EXPLAIN
    /// record's, and the plan is the same: made without the record's strings, bases and passed-over joins, each of
    /// which priced every unplaced join again (15% of `keeps::choose`).
    #[test]
    fn a_plans_step_rows_are_its_explain_records() {
        let prog = crate::code_rule_dsl::logic::rr::parse_version(
            ".decl a(x: number)\n.decl b(x: number, y: number)\n.decl c(y: number, z: number)\n.decl d(z: number, w: number)\n\
             .input a\n.input b\n.input c\n.input d\n.decl o(x: number, w: number)\n\
             o(x, w) :- a(x), b(x, y), c(y, z), d(z, w), !a(w), z != 3.\n\
             o(x, w) :- b(x, y), c(y, 7), d(7, w), w = x.\n\
             o(x, w) :- d(z, w), c(y, z), b(x, y), a(x).\n.output o\n",
        )
        .unwrap();
        let placed = body::Placed::whole(prog.clone());
        let model = model_with(&placed, &[("a", 50.0, &[50.0]), ("b", 100_000.0, &[1_000.0, 5_000.0]), ("c", 20_000.0, &[5_000.0, 300.0]), ("d", 9_000.0, &[300.0, 700.0])]);
        let explain = explaining(true);
        for (k, rule) in prog.rules().enumerate() {
            let b = body::nth(&prog, k);
            let r = prog.show_rule(rule);
            for first in [None, Some(1)] {
                let (plan, x) = model.plan_rule_explained(placed.fields().unwrap(), &b, first).unwrap();
                let (lean, rows) = model.plan_rule_step_rows(placed.fields().unwrap(), &b, first).unwrap();
                let recorded: Vector<f64> = x.expect("explained").steps.iter().map(|s| s.rows).collect();
                assert_eq!((b.shown(&lean.0), lean.1, lean.2), (b.shown(&plan.0), plan.1, plan.2), "{r}");
                assert_eq!(rows, recorded, "{r} from {first:?}");
            }
        }
        drop(explain);
    }
