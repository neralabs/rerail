use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Checkout {
    area: tempfile::TempDir,
    repo: PathBuf,
    policy: PathBuf,
}

impl Checkout {
    fn new() -> Self {
        let area = tempfile::tempdir().unwrap();
        let repo = area.path().join("checkout");
        let policy = area.path().join("identifiers.rr");
        fs::create_dir_all(repo.join("sub")).unwrap();
        fs::write(repo.join("a.py"), "x = 1\n").unwrap();
        fs::write(repo.join("sub/b.py"), "y = 2\n").unwrap();
        fs::write(&policy, ".policy \"ids\" revision 1\n.language python\n.violation Seen(at: node) \"identifier\"\nSeen(n) :- kind(n, \"identifier\").\n").unwrap();
        let fixture = Self { area, repo, policy };
        fixture.git(&["init", "-q"]);
        fixture.git(&["add", "a.py", "sub/b.py"]);
        fixture.git(&["commit", "-qm", "two identifiers"]);
        fixture
    }

    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(&self.repo)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .trim_end()
            .to_owned()
    }

    fn run(&self, name: &str, paths: &[&Path], scope: Option<&str>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rerail-engine"));
        command
            .env("RERAIL_TAPE_CACHE", self.area.path().join("store.sqlite"))
            .arg(name)
            .args(paths);
        if let Some(scope) = scope {
            command.args(["--revision", "HEAD", "--analysis-scope", scope]);
        }
        command.output().unwrap()
    }

    fn owners(&self, scope: &str) -> i64 {
        let db = rusqlite::Connection::open_with_flags(
            self.area.path().join("store.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let pending: i64 = db
            .query_row("SELECT count(*) FROM analysis_scope_pending", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(pending, 0);
        db.query_row(
            "SELECT count(*) FROM analysis_scope_manifest WHERE scope=?1",
            [format!("named:{scope}")],
            |row| row.get(0),
        )
        .unwrap()
    }
}

fn json(output: Output, code: i32) -> serde_json::Value {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "invalid CLI report: {e}\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn failed_batch_reports_each_policy_in_argument_order() {
    let f = Checkout::new();
    let out = f.area.path().join("batch");
    let broken = f.area.path().join("bad references.rr");
    fs::write(&broken, ".policy \"broken\" revision 1\n.language python\n.references {\"unknownParameter\":true}\n.violation Edge(at: node) \"edge\"\nEdge(n) :- reference_edge(n,_,_).\n").unwrap();
    for scope in [Some("batch"), None] {
        for bad_first in [true, false] {
            let (first, second) = if bad_first {
                (&broken, &f.policy)
            } else {
                (&f.policy, &broken)
            };
            let report = json(
                f.run("run-policies", &[&f.repo, &out, first, second], scope),
                1,
            );
            assert_eq!(report["status"], "failed");
            assert_eq!(
                report["diagnostics"].as_array().unwrap().len(),
                2,
                "{report}"
            );
            let runs = report["runs"].as_array().unwrap();
            assert_eq!(runs.len(), 2);
            assert_eq!(runs[0]["path"], first.to_str().unwrap());
            assert_eq!(runs[1]["path"], second.to_str().unwrap());
            let (failed_run, good_run) = if bad_first {
                (&runs[0], &runs[1])
            } else {
                (&runs[1], &runs[0])
            };
            assert!(
                failed_run["error"]
                    .as_str()
                    .unwrap()
                    .contains("unknownParameter")
            );
            assert!(good_run.get("error").is_none());
        }
    }
    assert_eq!(f.owners("batch"), 0);
    let complete = json(
        f.run("run-policies", &[&f.repo, &out, &f.policy], Some("batch")),
        0,
    );
    assert_eq!(complete["status"], "completed");
    assert_eq!(complete["diagnostics"].as_array().unwrap().len(), 2);
    assert!(f.owners("batch") > 0);
}

#[test]
fn incomplete_source_reads_return_partial_results_and_a_failed_exit() {
    let f = Checkout::new();
    let tree = f.git(&["rev-parse", "HEAD:sub"]);
    let object = f
        .repo
        .join(".git/objects")
        .join(&tree[..2])
        .join(&tree[2..]);
    let held = f.area.path().join("held-tree");
    let out = f.area.path().join("out");
    fs::rename(&object, &held).unwrap();
    for name in ["run-rr", "run-policies"] {
        let paths = if name == "run-rr" {
            [&f.policy, &f.repo, &out]
        } else {
            [&f.repo, &out, &f.policy]
        };
        let report = json(
            f.run(name, &paths.map(PathBuf::as_path), Some("partial")),
            1,
        );
        assert_eq!(report["status"], "failed");
        assert_eq!(report["sourceFailures"][0]["path"], "sub");
        assert_eq!(report["diagnostics"].as_array().unwrap().len(), 1);
        assert_eq!(report["diagnostics"][0]["path"], "a.py");
    }
    let logic = f.run("logic", &[&f.policy, &f.repo, &out], Some("partial"));
    assert_eq!(logic.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&logic.stderr).contains(&tree));
    assert!(!logic.stdout.is_empty());
    assert_eq!(f.owners("partial"), 0);

    let metadata = f.area.path().join("metadata.rr");
    fs::write(&metadata, "#include \"git.rr\"\n.policy \"meta\" revision 1\n.violation Commit(at: span) \"commit\"\nCommit(@place(\"git\", 1)) :- git_source_commit(_, _).\n").unwrap();
    let complete = json(
        f.run("run-rr", &[&metadata, &f.repo, &out], Some("metadata")),
        0,
    );
    assert_eq!(complete["status"], "completed");
    assert_eq!(complete["diagnostics"].as_array().unwrap().len(), 1);
    fs::rename(held, object).unwrap();
    let restored = json(
        f.run("run-rr", &[&f.policy, &f.repo, &out], Some("partial")),
        0,
    );
    assert_eq!(restored["diagnostics"].as_array().unwrap().len(), 2);
    assert_eq!(restored["status"], "completed");
    assert!(
        f.run("logic", &[&f.policy, &f.repo, &out], Some("partial"))
            .status
            .success()
    );
    assert!(f.owners("partial") > 0);
}
