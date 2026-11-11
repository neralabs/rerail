use super::*;
use crate::code_rule_dsl::{logic::{front, plan}, typing::file_local::Binding};

#[test]
fn fresh_and_restored_programs_have_identical_relation_identities() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("a.py"), "def f(x):\n    return x\n").unwrap();
    std::fs::write(root.join("b.py"), "from a import f\ny = f(1)\n").unwrap();
    let typeshed = root.join("typeshed/stdlib");
    std::fs::create_dir_all(&typeshed).unwrap();
    std::fs::write(typeshed.join("VERSIONS"), "builtins: 3.0-\n").unwrap();
    std::fs::write(typeshed.join("builtins.pyi"), "class object: ...\nclass int(object): ...\n").unwrap();
    let mut snapshots = Vec::new();
    for phase in ["fresh", "restored", "restored-again"] {
        let output = std::process::Command::new("nice").args(["-n", "19"])
            .arg(std::env::current_exe().unwrap())
            .args(["prepared_identity_child", "--ignored", "--nocapture", "--test-threads=1"])
            .env("RERAIL_PREPARED_IDENTITY_ROOT", root)
            .env("RERAIL_PREPARED_IDENTITY_PHASE", phase)
            .env("RERAIL_TAPE_CACHE", root.join("facts.sqlite"))
            .env("RERAIL_COMPOSE", "0").env("RERAIL_THREADS", "4").env("RUST_MIN_STACK", "67108864")
            .output().unwrap();
        assert!(output.status.success(), "{phase}: {}", String::from_utf8_lossy(&output.stdout).to_string() + &String::from_utf8_lossy(&output.stderr));
        let snapshot: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(format!("{phase}.json"))).unwrap()).unwrap();
        snapshots.push(snapshot);
    }
    assert!(!snapshots[0]["outputs"].as_str().unwrap().is_empty(), "the fixture must find a call");
    for next in &snapshots[1..] {
        assert_eq!(next["outputs"], snapshots[0]["outputs"]);
        let programs = next["programs"].as_object().unwrap();
        assert!(!programs.is_empty());
        for (key, program) in programs {
            assert_eq!(Some(program), snapshots[0]["programs"].get(key), "restored program {key} must retain its rules and identities");
        }
        assert_eq!(next["prepared"], "store");
        let phases = next["phases"].as_array().unwrap();
        assert!(phases.iter().any(|p| p["name"] == "struct"), "fixture must exercise the structural phase");
        assert!(phases.iter().any(|p| p["name"] == "types"), "fixture must exercise the typing phase");
        for phase in phases.iter().filter(|p| p["name"] == "struct" || p["name"] == "types") {
            assert_eq!(phase["prepared"], "store", "phase preparation must be restored: {phase}");
            assert_eq!(phase["kept"][0], 0, "unchanged phase strata must not run again: {phase}");
        }
    }
}

#[test]
#[ignore = "child process for fresh versus restored preparation"]
fn prepared_identity_child() {
    let root = std::path::PathBuf::from(std::env::var("RERAIL_PREPARED_IDENTITY_ROOT").unwrap());
    let phase = std::env::var("RERAIL_PREPARED_IDENTITY_PHASE").unwrap();
    let out = root.join("out");
    let program = crate::code_rule_dsl::tools::program_text(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/typing/programs/calls_p3.rr")
    ).unwrap();
    let text = format!("{program}\n.decl hit(c:node,f:node)\nhit(c,f) :- call_target(c,f).\n.output hit\n");
    let report = super::super::run_with_options(&[text.as_str()], &root, None, &out, "program", Binding::Cpu,
        1 << 18, &super::super::Typing::default(), &serde_json::json!({
            "inputFiles":["a.py","b.py"], "env":[format!("python.typeshed={}", root.join("typeshed").display())]
        })).unwrap();
    let mut programs = BTreeMap::new();
    super::super::with_prepared(|kept| for (key, all) in kept.iter() {
        for p in all {
            let (sccs, _) = front::strata(&p.split.cross).unwrap();
            let by = plan::rules_by_stratum(&p.split.cross, &sccs);
            let identities: BTreeMap<_, _> = plan::identities(&p.split.cross, &sccs, &by).into_iter().collect();
            let label = key.iter().map(|b| format!("{b:02x}")).collect::<String>();
            assert!(!programs.contains_key(&label), "one store must have one prepared variant per key");
            programs.insert(label, serde_json::json!({"outputs":p.prog.outputs,"front":format!("{:?}",p.prog),
                "local":format!("{:?}",p.split.local), "cross":format!("{:?}",p.split.cross),
                "identities":identities}));
        }
    });
    let phases: Vec<_> = report.phase_runs.iter().map(|p| serde_json::json!({"name":p.name,
        "prepared":p.report.prepared,"kept":p.report.cross_kept,"maintained":p.report.cross_maintained,"memo":p.report.memo,"chunks":p.report.chunks,"strata":p.report.cross_strata})).collect();
    let result = serde_json::json!({"programs":programs,"outputs":std::fs::read_to_string(out.join("hit.csv")).unwrap(),
        "prepared":report.prepared,"phases":phases});
    std::fs::write(root.join(format!("{phase}.json")), serde_json::to_vec_pretty(&result).unwrap()).unwrap();
}
