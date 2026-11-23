    use crate::code_rule_dsl::typing::file_local::Binding;

    /// An unchanged run is kept; a document's edit, a new document, the root's settings (read by `env`) and the text
    /// each run it again.
    #[test]
    fn an_unchanged_run_writes_its_kept_outputs() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("reuse");
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.py"), "def f():\n    return 1\n").unwrap();
        let out = dir.join("out");
        super::super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let program = |name: &str| {
            let text = format!("#include \"core.rerail\"\n.decl {name}(s: symbol)\n{name}(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output {name}\n");
            crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_version(&text).unwrap()).unwrap()
        };
        let options = serde_json::json!({ "reuse": true });
        let run = |text: &str| super::super::run_with_options(&[text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options).unwrap();
        let kept = |r: &super::super::Report| r.phases.iter().any(|p| p.0.starts_with("unchanged"));
        let defs = || crate::code_rule_dsl::logic::plan::output::shown(&out, "defs").unwrap();
        let text = program("defs");
        assert!(!kept(&run(&text)), "the first run derives");
        assert_eq!(defs(), [["f"]]);
        std::fs::remove_file(out.join(crate::code_rule_dsl::logic::plan::output::FILE)).unwrap();
        let again = run(&text);
        assert!(kept(&again), "unchanged: kept");
        assert_eq!((defs(), again.rows.get("defs").copied()), (vec![vec!["f".to_owned()]], Some(1)), "the kept outputs written again");
        // A document's edit runs it again, and that run is kept in turn.
        std::fs::write(src.join("a.py"), "def g():\n    return 1\n").unwrap();
        assert!(!kept(&run(&text)), "an edited document");
        assert_eq!(defs(), [["g"]]);
        assert!(kept(&run(&text)));
        std::fs::write(src.join("b.py"), "def h():\n    pass\n").unwrap();
        assert!(!kept(&run(&text)), "a new document");
        assert_eq!(defs(), [["g"], ["h"]]);
        std::fs::write(src.join("rerail.env"), "mode=strict\n").unwrap();
        run(&text);
        assert!(kept(&run(&text)), "settings reach only a program that declares `env`");
        let with_env = |name: &str| program(name).replace(".output", ".decl env(k: symbol, v: symbol)\n.output");
        assert!(!kept(&run(&with_env("defs"))));
        assert!(kept(&run(&with_env("defs"))));
        std::fs::write(src.join("rerail.env"), "mode=lax\n").unwrap();
        assert!(!kept(&run(&with_env("defs"))), "the root's settings, for a program that declares `env`");
        assert!(!kept(&run(&program("names"))), "another text");
        // Each library file a text includes is in the record, with its digest (a flattened text includes none).
        let rules = super::rule_files(&super::super::Texts::of(&["#include \"core.rerail\"\n.decl x(s: symbol)\n"]).unwrap(), &serde_json::Value::Null).unwrap();
        assert!(rules.iter().any(|r| r.1.ends_with("core.rerail") && !r.2.is_empty()) && rules.iter().any(|r| r.1.ends_with("components.rerail")), "{rules:?}");
        // A policy file the program reads (`.input … filename=`) is consulted: its edit runs it again.
        std::fs::write(src.join("allow.txt"), "a\n").unwrap();
        let reads = "#include \"core.rerail\"\n.decl entry(n: number, e: symbol)\n.input entry(IO=lines, filename=\"allow.txt\")\n.decl listed(e: symbol)\nlisted(e) :- entry(_, e).\n.output listed\n";
        let reads = crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_version(reads).unwrap()).unwrap();
        run(&reads);
        assert!(kept(&run(&reads)));
        std::fs::write(src.join("allow.txt"), "a\nb\n").unwrap();
        assert!(!kept(&run(&reads)), "an edited input file");
        assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out, "listed").unwrap(), [["a"], ["b"]]);
        assert!(kept(&run(&reads)));
        // Off: derived every time.
        let off = serde_json::json!({});
        let r = super::super::run_with_options(&[&text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &off).unwrap();
        assert!(!kept(&r), "not asked");
        super::super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A phase program the record found among the engine's own is checked where it was looked for first, as a file that must
    /// still be missing: one written into the run's `phase_dir` is read instead, so the record's rule files differ
    /// (decision 0275: the check compares the record's include graphs, not the rule files listed again).
    #[test]
    fn a_phase_program_appearing_where_it_was_looked_for_first_differs() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("reuse-phase");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let options = serde_json::json!({ "phase_dir": dir.to_string_lossy() });
        let text = "#include \"core.rerail\"\n.decl values__x(s: symbol)\n";
        let rules = || super::rule_files(&super::super::Texts::of(&[text]).unwrap(), &options).unwrap();
        let shadow = dir.join("values.rerail");
        let entry = |rules: &[(String, String, String)]| rules.iter().find(|r| r.1 == shadow.to_string_lossy()).cloned();
        let before = rules();
        assert_eq!(entry(&before).map(|r| r.2), Some(String::new()), "looked for there first, and missing: {before:?}");
        assert!(before.iter().any(|r| std::path::Path::new(&r.1).ends_with("typing/programs/values.rerail") && !r.2.is_empty()), "the engine's own, read: {before:?}");
        std::fs::write(&shadow, "#include \"core.rerail\"\n.decl y(s: symbol)\n").unwrap();
        // The check compares each `rule` line's digest with the file's now: an empty one with a file there differs.
        let now = rules();
        assert!(entry(&now).is_some_and(|r| !r.2.is_empty()), "the file there is read now: {now:?}");
        assert!(!now.iter().any(|r| std::path::Path::new(&r.1).ends_with("typing/programs/values.rerail")), "and the engine's own is not: {now:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Read off phase 0's graph (no `phase_dir`), a phase program a text names and the engine does not have is listed
    /// where it was looked for, with no digest, as `listed` lists it (decision 0275): one written there differs.
    #[test]
    fn a_phase_program_the_graph_did_not_find_is_listed_as_missing() {
        let text = "#include \"core.rerail\"\n.decl nosuchphase__x(s: symbol)\n";
        let texts = [text];
        let given = super::super::Texts::of(&texts).unwrap();
        assert!(given.graphs.iter().all(Option::is_some), "the text has its graph");
        let rules = super::rule_files(&given, &serde_json::json!({})).unwrap();
        let missing: Vec<_> = rules.iter().filter(|r| std::path::Path::new(&r.1).ends_with("typing/programs/nosuchphase.rerail")).collect();
        assert_eq!(missing.len(), 1, "{rules:?}");
        assert!(missing[0].2.is_empty(), "no digest: {rules:?}");
    }

    #[test]
    fn unchanged_cross_file_outputs_skip_derivation_and_changed_members_invalidate() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("cross-reuse");
        let src = dir.join("src"); std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.py"),"x = 1\n").unwrap();
        std::fs::write(src.join("b.py"),"y = 2\n").unwrap();
        super::super::TEST_TAPE_CACHE.with(|p|*p.borrow_mut()=Some(dir.join("store.sqlite")));
        let p = "#include \"core.rerail\"\n.decl pairs(a:symbol,b:symbol)\npairs(a,b) :- doc(d,a,_),doc(e,b,_),d!=e.\n.output pairs\n";
        let out=dir.join("out");
        let run=||super::super::run_with_options(&[p],&src,None,&out,"program",Binding::Cpu,1<<18,&Default::default(),&serde_json::json!({"reuse":true})).unwrap();
        let first=run();
        let original=crate::code_rule_dsl::logic::plan::output::shown(&out,"pairs").unwrap();
        assert_eq!(first.rows["pairs"],2);
        std::fs::remove_file(out.join(crate::code_rule_dsl::logic::plan::output::FILE)).unwrap();
        let kept=run();
        assert!(kept.phases.iter().any(|p|p.0.starts_with("unchanged")),"{:?}",kept.phases);
        assert_eq!(crate::code_rule_dsl::logic::plan::output::shown(&out,"pairs").unwrap(),original);
        assert_eq!(kept.chunks,0);
        std::fs::write(src.join("c.py"),"z = 3\n").unwrap();
        {
            // An older program's output, written over the stack: an image of it alone, over the one there.
            use crate::code_rule_dsl::logic::plan::output;
            let outputs=output::Outputs::open(&out).unwrap();
            let mut image=output::Builder::new(outputs.images().last().map_or(0,output::Image::flags),true);
            let mut stale=output::Rows::new(vec![output::col::TEXT]);
            stale.push(&[output::Cell::Text("old")]);
            image.put("stale",&[&stale],&[]);
            let bytes=image.finish();
            drop(outputs);
            output::write(&out,&bytes).unwrap();
        }
        assert_eq!(run().rows["pairs"],6);
        let kept=run();
        assert!(!kept.rows.contains_key("stale"),"an older program's output is not part of this run");
        std::fs::remove_file(src.join("b.py")).unwrap();
        assert_eq!(run().rows["pairs"],2);
        super::super::TEST_TAPE_CACHE.with(|p|*p.borrow_mut()=None);
        let _=std::fs::remove_dir_all(dir);
    }

    #[test]
    fn outputs_and_records_round_trip() {
        let files = vec![("a.rro".to_owned(), b"1\t2\n".to_vec()), ("b.rro".to_owned(), Vec::new())];
        assert_eq!(super::decode_outputs(&super::encode_outputs(&files)).unwrap(), files);
        let c = vec![("doc".to_owned(), "a.py".to_owned(), "ab".to_owned()), ("rule".to_owned(), "/x/core.rerail".to_owned(), "cd".to_owned())];
        assert_eq!(super::decode_consulted(&super::encode_consulted(&c)), Some(c.clone()));
        let bytes = super::encode_consulted(&c);
        assert_eq!(super::decode_consulted(&bytes[..bytes.len() - 1]), None, "a cut record is none");
    }

    /// A run measured as it goes (the heap sampler's switches, a profile, a cost store and an image store of its own,
    /// a trace) is the same run as one that is not: the warm run after a cold one sampled for its heap is kept. The
    /// cold runs of a measurement passed `RERAIL_HEAP_SAMPLE` and `RERAIL_HEAP_AT` and the warm runs did not, so every
    /// warm run looked its record up under a key no cold run wrote, and ran phase 1 again (9 s of CPU against 0.2 s).
    /// A switch that changes what a run outputs still runs it again.
    #[test]
    fn a_run_measured_as_it_goes_keeps_the_key_of_one_that_is_not() {
        let dir = crate::code_rule_dsl::fixtures::temp().with_extension("reuse-measured");
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.py"), "def f():\n    return 1\n").unwrap();
        let out = dir.join("out");
        super::super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = Some(dir.join("store.sqlite")));
        let text = {
            let text = "#include \"core.rerail\"\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output defs\n";
            crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_version(text).unwrap()).unwrap()
        };
        let options = serde_json::json!({ "reuse": true });
        let run = || super::super::run_with_options(&[&text], &src, None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options).unwrap();
        let kept = |r: &super::super::Report| r.phases.iter().any(|p| p.0.starts_with("unchanged"));
        let costs = dir.join("costs.sqlite");
        let images = dir.join("images");
        {
            let _measured = crate::code_rule_dsl::logic::plan::corpus::switches::set(&[
                ("RERAIL_HEAP_SAMPLE", "1"),
                ("RERAIL_HEAP_AT", "1500"),
                ("RERAIL_HEAP_FRAMES", "1"),
                ("RERAIL_PROF_MEMORY_MS", "5"),
                ("RERAIL_TIMELINE", "1"),
                ("RERAIL_COSTS", costs.to_str().unwrap()),
                ("RERAIL_IMAGE_STORE", images.to_str().unwrap()),
                ("RERAIL_FRAMES_TRACE", "1"),
            ]);
            assert!(!kept(&run()), "the cold run derives");
        }
        assert!(kept(&run()), "the warm run, measured by none of them, is the cold run's");
        {
            let _other = crate::code_rule_dsl::logic::plan::corpus::switches::set(&[("RERAIL_AST_PARTS", "off")]);
            assert!(!kept(&run()), "a switch that changes the run's outputs runs it again");
        }
        super::super::TEST_TAPE_CACHE.with(|p| *p.borrow_mut() = None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every switch a profile or the heap sampler reads only measures the run, so none is in a run's key: a new one
    /// read there is left out of it without being listed (`reuse::measures_only`).
    #[test]
    fn every_switch_a_profile_reads_is_left_out_of_the_runs_key() {
        let root = crate::source_tree::native();
        // The profile's sources (`logic/profile.rs` and its directory, in whichever crates keep them) and the heap
        // sampler's (rerail-prof's `lib.rs`; the crate's other files are the engine's, moved there).
        let dirs = crate::source_tree::source_paths("code_rule_dsl/logic/profile");
        let mut files = vec![crate::source_tree::source_file("code_rule_dsl/logic/profile.rs"), root.join("crates/prof/src/lib.rs")];
        for d in dirs {
            let mut stack = vec![d];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d).unwrap() {
                    let p = e.unwrap().path();
                    if p.is_dir() { stack.push(p) } else if p.extension().is_some_and(|x| x == "rs") { files.push(p) }
                }
            }
        }
        let re = regex::Regex::new(r#""(RERAIL_[A-Z0-9_]+)""#).unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for f in &files {
            let text = std::fs::read_to_string(f).unwrap();
            for c in re.captures_iter(&text) {
                seen.insert(c[1].to_owned());
            }
        }
        assert!(seen.contains("RERAIL_HEAP_SAMPLE") && seen.contains("RERAIL_HEAP_AT"), "the sampler's switches are found: {seen:?}");
        // A dump turns the cutoff off (`reuse::wanted`); every other switch read there must be left out of the key.
        let keyed: Vec<&String> = seen.iter().filter(|s| !s.starts_with("RERAIL_DUMP") && !super::measures_only(s)).collect();
        assert!(keyed.is_empty(), "read by a profile and in a run's key: {keyed:?}");
    }

    /// A run over a Git snapshot is kept as a run over files is: what it read is the selected tree's (its blobs, its
    /// revisions), each checked as the record has it. A named analysis's view is another path in each run (its
    /// invocation's), so the record's key names the root by the repository's facts and the selected object. A new
    /// commit runs it again, and that run is kept in turn. Before, every snapshot run evaluated again, warm or not.
    #[test]
    fn an_unchanged_git_snapshot_run_writes_its_kept_outputs() {
        use crate::content_store::{ContentStore, analysis_scope::AnalysisRun};
        use crate::git_workspace::snapshot::Snapshot;
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("a.py"), "def f():\n    return 1\n").unwrap();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgSign=false"])
                .args(args).current_dir(&repo).output().unwrap();
            assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
        };
        git(&["init", "-q"]);
        git(&["add", "--", "a.py"]);
        git(&["commit", "-qm", "one"]);
        let text = {
            let text = "#include \"core.rerail\"\n.decl defs(s: symbol)\ndefs(x) :- kind(n, \"function_definition\"), field(n, \"name\", i), text(i, x).\n.output defs\n";
            crate::code_rule_dsl::logic::rr::print_version(&crate::code_rule_dsl::logic::rr::parse_version(text).unwrap()).unwrap()
        };
        let store = dir.path().join("store.sqlite");
        let options = serde_json::json!({ "reuse": true });
        let run = |id: &str| {
            let snapshot = Snapshot::scoped(&repo, "HEAD", "reuse").unwrap();
            let mut analysis = AnalysisRun::begin(ContentStore::open(&store).unwrap(), "named:reuse", id).unwrap();
            let out = dir.path().join(id);
            let report = super::super::with_tape_cache_path(&store, Some((snapshot.root().to_owned(), analysis.owner.clone())), || {
                super::super::run_with_options(&[&text], snapshot.root(), None, &out, "program", Binding::Cpu, 1 << 18, &Default::default(), &options).unwrap()
            });
            analysis.finish(true).unwrap();
            let kept = report.phases.iter().any(|p| p.0.starts_with("unchanged"));
            (kept, crate::code_rule_dsl::logic::plan::output::shown(&out, "defs").unwrap())
        };
        let one = || vec![vec!["f".to_owned()]];
        assert_eq!(run("cold"), (false, one()), "the first run derives");
        assert_eq!(run("warm"), (true, one()), "unchanged: kept");
        std::fs::write(repo.join("a.py"), "def g():\n    return 1\n").unwrap();
        git(&["commit", "-qam", "two"]);
        let two = vec![vec!["g".to_owned()]];
        assert_eq!(run("edited"), (false, two.clone()), "a new commit");
        assert_eq!(run("again"), (true, two), "kept in turn");
        rerail_accel::upkeep::retire_files_under(dir.path());
    }
