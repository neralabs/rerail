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

    fn settled_store(&self) -> rusqlite::Connection {
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
        db
    }

    fn owners(&self, scope: &str) -> i64 {
        self.settled_store()
            .query_row(
                "SELECT count(*) FROM analysis_scope_manifest WHERE scope=?1",
                [format!("named:{scope}")],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn directory_memberships(&self) -> Vec<(i64, i64, i64, i64)> {
        let db = self.settled_store();
        let mut members = db.prepare(
            "SELECT manifest_id, acquired_at, acquisition, recorded_run FROM analysis_scope_manifest
             WHERE scope GLOB 'cwd:*' ORDER BY manifest_id",
        ).unwrap();
        members
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
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
fn revision_outputs_include_literal_and_folded_facts_even_without_documents() {
    let f = Checkout::new();
    let out = f.area.path().join("export");
    let program = f.area.path().join("export.rr");
    let literal = ".language python\n.decl literal(value: symbol)\nliteral(\"from-fact\").\n.output literal\n";
    let marker = r#".decl marker(value: symbol)
marker("from-fact").
marker(s) :- file_line("marker", _, s).
.output marker
"#;
    let joined = r#".decl per_document(value: symbol)
per_document(s) :- marker(s), kind(_, "identifier").
.output per_document
"#;
    fs::write(f.repo.join("marker"), "from-blob\n").unwrap();
    f.git(&["add", "marker"]);
    f.git(&["commit", "-qm", "marker"]);
    let check = |source: &str, expected: &[(&str, &[&str])]| {
        fs::write(&program, source).unwrap();
        let run = f.run("logic", &[&program, &f.repo, &out], Some("exports"));
        assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
        assert!(f.run("csv", &[&out], None).status.success());
        for &(relation, expected) in expected {
            let text = fs::read_to_string(out.join(format!("{relation}.csv"))).unwrap();
            let mut rows = text.lines().collect::<Vec<_>>();
            rows.sort_unstable();
            assert_eq!(rows, expected, "{relation} in {source}");
        }
    };
    let mixed = [literal, marker].concat();
    check(literal, &[("literal", &["from-fact"])]);
    check(&mixed, &[("marker", &["from-blob", "from-fact"])]);
    check(&[&mixed, joined].concat(), &[("per_document", &["from-blob", "from-fact"])]);
    f.git(&["rm", "-q", "--", "a.py", "sub/b.py", "marker"]);
    f.git(&["commit", "-qm", "empty tree"]);
    check(&mixed, &[("literal", &["from-fact"]), ("marker", &["from-fact"])]);
}

#[test]
fn nested_rule_evaluations_preserve_existing_output_files() {
    let f = Checkout::new();
    let out = f.area.path().join("output");
    let constants = out.join("constants");
    fs::create_dir_all(&constants).unwrap();
    let sentinel = constants.join("keep.txt");
    fs::write(&sentinel, "user file").unwrap();
    let typing = out.join("typing-calls");
    fs::create_dir(&typing).unwrap();
    fs::write(typing.join("keep.txt"), "typing file").unwrap();
    fs::write(f.repo.join("call.py"), "unknown_call()\n").unwrap();
    f.git(&["add", "call.py"]);
    f.git(&["commit", "-qm", "call for the typing program"]);
    let rule = f.area.path().join("settings.rr");
    fs::write(&rule, r#"#include "python/calls.rr"
.decl label(s: symbol)
label(s) :- file_line("a.py", _, s).
.decl found(s: symbol)
found(s) :- label(s), call_open(n).
.output found
"#).unwrap();
    let result = f.run("logic", &[&rule, &f.repo, &out], Some("constants"));
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(fs::read_to_string(sentinel).unwrap(), "user file");
    assert_eq!(fs::read_to_string(typing.join("keep.txt")).unwrap(), "typing file");
    assert!(f.run("csv", &[&out], None).status.success());
    assert_eq!(fs::read_to_string(out.join("found.csv")).unwrap(), "x = 1\n");
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
    assert!(f.directory_memberships().is_empty());
    let complete = json(
        f.run("run-policies", &[&f.repo, &out, &f.policy], Some("batch")),
        0,
    );
    assert_eq!(complete["status"], "completed");
    assert_eq!(complete["diagnostics"].as_array().unwrap().len(), 2);
    assert!(f.owners("batch") > 0);
}

#[test]
fn standalone_logic_publishes_each_source_after_its_history_query_finishes() {
    let f = Checkout::new();
    let policy = f.area.path().join("history.rr");
    let out = f.area.path().join("rows");
    fs::write(&policy, "#include \"analysis.rr\"\n.decl previous(r: symbol)\nprevious(r) :- analysis_scope_run(_,r).\n.output previous\n").unwrap();
    for (count, scope) in [None, Some("review"), None].into_iter().enumerate() {
        let result = f.run("logic", &[&policy, &f.repo, &out], scope);
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        assert!(String::from_utf8_lossy(&result.stdout).contains(&format!("previous: {count} rows")));
        let db = f.settled_store();
        assert_eq!(db.query_row("SELECT count(*) FROM analysis_scope_run", [], |r| r.get::<_, i64>(0)).unwrap(), count as i64 + 1);
    }
    let db = f.settled_store();
    let commit = f.git(&["rev-parse", "HEAD"]);
    assert_eq!(db.query_row("SELECT git_commit FROM analysis_scope_run WHERE scope='named:review'", [], |r| r.get::<_, String>(0)).unwrap(), commit);
    assert_eq!(db.query_row("SELECT count(*) FROM analysis_scope_run WHERE directory IS NOT NULL", [], |r| r.get::<_, u32>(0)).unwrap(), 2);
}

#[test]
fn ordered_commit_trailers_match_git_for_each_requested_message() {
    let f = Checkout::new();
    let message = f.area.path().join("message.txt");
    let policy = f.area.path().join("trailers.rr");
    let out = f.area.path().join("trailers");
    let mut source = String::from(r#"#include "git.rr"
.decl requested(commit: symbol)
git_object_request(r, c) :- git_root_repo(r), requested(c).
.decl result(commit: symbol, i: number, name: symbol, value: symbol)
result(c, i, k, v) :- git_root_repo(r), requested(c), git_trailer(r, c, i, k, v).
.output result
"#);
    let mut expected = Vec::new();
    for body in [
        "Key: title only\n",
        "Subject\nKey: still in the title\n",
        "Subject\n\nLabel: in the body\n\nThis is the last paragraph.\n",
        "Subject\n\nKey: earlier\n\nReview : first\n\tpart two\n   part three\nReview: first\nReview: first\nEmpty:\n",
        "Subject\n\nOrdinary prose\nReview: no trailer block\n",
        "Subject\n\nline one\nline two\nline three\nSigned-off-by: Alice\n",
        "Subject\n\nline one\nline two\nline three\nline four\nSigned-off-by: Alice\n",
        "Subject\n\nReview: before comment\n# internal comment\nCase: \u{03bb}\n\n# trailing comment\n",
        "Subject\n\nfolded-text: caf\u{e9}  \n  suite\nlast: value: with colon",
        "Subject\n\nReview: before scissors\n# ------------------------ >8 ------------------------\nFake: after scissors\n",
        "Subject\n\nKey: before conflicts\n\nConflicts:\n\ta.py\n",
        "Subject\n\n(cherry picked from commit abcd)\nordinary\nConflicts:\n   \n",
        "Subject\n\n---\n\nKey: after divider\n",
        "Subject\n\nnoise\nReview: via cherry-pick\n(cherry picked from commit abcd)\n",
    ] {
        fs::write(&message, body).unwrap();
        let commit = f.git(&["commit-tree", "HEAD^{tree}", "-F", message.to_str().unwrap()]);
        source.push_str(&format!("requested(\"{commit}\").\n"));
        let parsed = f.git(&["interpret-trailers", "--parse", "--no-divider", message.to_str().unwrap()]);
        for (i, line) in parsed.lines().enumerate() {
            let (name, value) = line.split_once(':').unwrap();
            expected.push(format!("{commit}\t{i}\t{name}\t{}", value.strip_prefix(' ').unwrap_or(value)));
        }
    }
    expected.sort();
    let tree = f.git(&["rev-parse", "HEAD^{tree}"]);
    fs::remove_file(f.repo.join(".git/objects").join(&tree[..2]).join(&tree[2..])).unwrap();
    fs::write(&policy, &source).unwrap();
    let run = f.run("logic", &[&policy, &f.repo, &out], Some("trailers"));
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(f.run("csv", &[&out], None).status.success());
    let csv = fs::read_to_string(out.join("result.csv")).unwrap();
    let mut actual: Vec<_> = csv.lines().collect();
    actual.sort_unstable();
    assert_eq!(actual, expected);

    source.push_str(".decl filename(i: number, word: symbol)\nfilename(i, word) :- doc(_, path, _), match_all(\"(?<word>[a-z]+)\", path, i, _, _).\n.output filename\n");
    fs::write(&policy, source).unwrap();
    let with_documents = f.run("logic", &[&policy, &f.repo, &out], Some("trailers"));
    assert!(!with_documents.status.success());
    assert!(String::from_utf8_lossy(&with_documents.stderr).contains(&tree));
}

#[test]
fn git_location_rules_identify_the_common_directory_and_each_open_worktree() {
    let f = Checkout::new();
    let first = f.area.path().join("first link");
    let second = f.area.path().join("second link");
    for dir in [&first, &second] {
        f.git(&["worktree", "add", "--detach", dir.to_str().unwrap()]);
    }
    let bare = f.area.path().join("bare.git");
    f.git(&["clone", "-q", "--bare", ".", bare.to_str().unwrap()]);
    let rules = f.area.path().join("locations.rr");
    let out = f.area.path().join("locations");
    fs::create_dir(f.area.path().join("includes")).unwrap();
    fs::write(f.area.path().join("includes/local.rr"), "#include \"../common.rr\"\n").unwrap();
    fs::write(f.area.path().join("common.rr"), "#include \"git.rr\"\n").unwrap();
    fs::write(&rules, r#"#include "includes/local.rr"
.decl dir(path: symbol)
dir(p) :- git_root_repo(r), git_repository_dir(r, p).
.decl work(path: symbol)
work(p) :- git_root_view(_, v), git_worktree(v, p).
.decl repo(r: symbol)
repo(r) :- git_repository_dir(r, _).
.decl view(v: symbol)
view(v) :- git_root_view(_, v).
.output dir, work, repo, view
"#).unwrap();
    let read = |root: &Path, revision: bool| {
        let result = f.run("logic", &[&rules, root, &out], revision.then_some("locations"));
        assert!(result.status.success(), "{} (revision={revision}): {}", root.display(), String::from_utf8_lossy(&result.stderr));
        assert!(f.run("csv", &[&out], None).status.success());
        ["dir", "work", "repo", "view"].map(|name| fs::read_to_string(out.join(format!("{name}.csv"))).unwrap().trim_end().to_owned())
    };
    let path = |p: &Path| p.canonicalize().unwrap().to_str().unwrap().to_owned();
    let [common, work, repo, main_view] = read(&f.repo, true);
    assert_eq!(common, path(&f.repo.join(".git")));
    assert_eq!(work, path(&f.repo));
    assert!(!repo.is_empty() && !main_view.is_empty());
    let mut views = std::collections::BTreeSet::from([main_view]);
    for linked in [&first, &second] {
        let [d, w, r, v] = read(linked, true);
        assert_eq!((&d, &r), (&common, &repo));
        assert_eq!(w, path(linked));
        assert!(views.insert(v.clone()));
        assert_eq!(read(linked, false), [d, w, r, v]);
    }
    let [d, w, r, v] = read(&bare, true);
    assert_eq!(d, path(&bare));
    assert!(w.is_empty());
    let moved = f.area.path().join("moved.git");
    fs::rename(bare, &moved).unwrap();
    assert_eq!(read(&moved, true), [path(&moved), String::new(), r, v]);
}

#[cfg(not(feature = "headless"))]
#[test]
fn fresh_and_reused_catalogs_inspect_the_same_policy_and_include_graph() {
    use rerail_engine::LoadedRrPolicies;
    use std::time::Instant;

    let area = tempfile::tempdir().unwrap();
    let policy = area.path().join("words.rr");
    fs::write(&policy, "#include \"git.rr\"\n.policy \"words\" revision 1\n.violation Seen(at: span) \"commit\"\nSeen(@place(\"git\", 1)) :- git_root_repo(_).\n").unwrap();
    let path = policy.to_str().unwrap();
    let store = area.path().join("store.sqlite");
    let open = || LoadedRrPolicies::new(Some(store.to_str().unwrap().to_owned()));
    let catalog = open();
    let expected = catalog.load(path.to_owned(), None).unwrap();
    let count = if rerail_prof::on() { 50 } else { 2 };
    for fresh in [false, true] {
        let inspect = || if fresh { open().load(path.to_owned(), None) } else { catalog.load(path.to_owned(), None) };
        assert_eq!(inspect().unwrap(), expected);
        let before = rerail_prof::stages().clone();
        let mut elapsed = Vec::new();
        for _ in 0..count {
            let start = Instant::now();
            let actual = inspect().unwrap();
            elapsed.push(start.elapsed());
            assert_eq!(actual, expected);
        }
        if rerail_prof::on() {
            elapsed.sort_unstable();
            eprintln!("fresh={fresh} median wall time {:?} ({count} inspections)", elapsed[count / 2]);
            let after = rerail_prof::stages().clone();
            let calls = |key: &str| after.get(key).map_or(0, |s| s.1) - before.get(key).map_or(0, |s| s.1);
            for (key, (total, _)) in &after {
                if key.starts_with("rr.inspect.") || key == "rr.stored_program_decode" {
                    let cpu = *total - before.get(key).map(|s| s.0).unwrap_or_default();
                    eprintln!("{key} {:?} CPU per inspection, {} calls", cpu / count as u32, calls(key));
                }
            }
            assert_eq!(calls("rr.program_reused"), count);
            assert_eq!(calls("rr.inspect.inputs"), count);
            assert_eq!(calls("rr.inspect.store"), if fresh { count } else { 0 });
            if std::env::var_os("RERAIL_PROGRAM_IMAGE").is_none() {
                assert_eq!(calls("rr.stored_program_decode"), 0);
            }
        }
    }
    rerail_accel::upkeep::retire_files_under(&area.path().canonicalize().unwrap());
}

#[test]
fn the_documented_head_queries_preserve_the_worktree_in_answers_and_errors() {
    let f = Checkout::new();
    let linked = f.area.path().join("linked");
    let linked_path = linked.to_str().unwrap();
    f.git(&["worktree", "add", "--detach", linked_path]);
    f.git(&["-C", linked_path, "commit", "--allow-empty", "--allow-empty-message", "-qm", ""]);
    let tips = [f.git(&["rev-parse", "HEAD"]), f.git(&["-C", linked_path, "rev-parse", "HEAD"])];
    assert_ne!(tips[0], tips[1]);
    let policy = f.area.path().join("head.rr");
    let out = f.area.path().join("facts");
    let evaluate = |source: &Path, program: &str, relation: &str| {
        fs::write(&policy, program).unwrap();
        let run = f.run("logic", &[&policy, source, &out], Some("shared"));
        assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
        let export = f.run("csv", &[&out], None);
        assert!(export.status.success(), "{}", String::from_utf8_lossy(&export.stderr));
        fs::read_to_string(out.join(format!("{relation}.csv"))).unwrap()
    };
    let example = |doc: &'static str, relation: &str| {
        doc.split("```rr\n").skip(1).filter_map(|block| block.split_once("```"))
            .map(|(program, _)| program).find(|program| program.lines().any(|line| line == format!(".output {relation}"))).unwrap()
    };
    let view_tip = example(include_str!("../../docs/rule-syntax/git.md"), "view_tip");
    let empty = example(include_str!("../../docs/final-shape/notes/git-inputs/library.md"), "empty_head_message");
    let main_row = evaluate(&f.repo, view_tip, "view_tip");
    let link_row = evaluate(&linked, view_tip, "view_tip");
    let main: Vec<_> = main_row.trim_end().split('\t').collect();
    let link: Vec<_> = link_row.trim_end().split('\t').collect();
    assert_eq!((main.len(), link.len()), (3, 3));
    assert_eq!(main[0], link[0]);
    assert_ne!(main[1], link[1]);
    assert_eq!([main[2], link[2]], [tips[0].as_str(), tips[1].as_str()]);
    assert_eq!(evaluate(&f.repo, empty, "empty_head_message"), "");
    assert_eq!(evaluate(&linked, empty, "empty_head_message"), link_row);

    let missing = format!("#include \"git.rr\"\ngit_view_revision_request(\"{}\", \"HEAD\").\ngit_view_revision_request(v,\"no-such-ref\") :- git_root_view(_, v).\n.output git_view_revision_error\n", main[1]);
    let errors = evaluate(&linked, &missing, "git_view_revision_error");
    assert!(errors.lines().any(|row| row == format!("{}\tHEAD\trepository-not-open", main[1])), "{errors}");
    assert!(errors.lines().any(|row| row == format!("{}\tno-such-ref\tmissing-revision", link[1])), "{errors}");
}

#[test]
fn a_failed_working_tree_batch_preserves_the_previous_directory_memberships() {
    let f = Checkout::new();
    let out = f.area.path().join("batch");
    let other = f.area.path().join("other.rr");
    fs::write(&other, ".policy \"other\" revision 1\n.language python\n.violation AlsoSeen(at: node) \"second identifier\"\nAlsoSeen(n) :- kind(n, \"identifier\").\n").unwrap();
    let bad = f.area.path().join("bad.rr");
    fs::write(&bad, ".policy \"invalid\" revision 1\n").unwrap();
    let good = || f.run("run-policies", &[&f.repo, &out, &f.policy, &other], None);
    let initial = json(good(), 0);
    assert_eq!(initial["diagnostics"].as_array().unwrap().len(), 4);
    assert_eq!(initial["runs"][0]["together"], 2, "{initial}");
    let before = f.directory_memberships();
    assert!(!before.is_empty());
    fs::write(f.repo.join("a.py"), "x = y\n").unwrap();
    for policies in [[&bad, &f.policy, &other], [&f.policy, &other, &bad]] {
        let [first, second, third] = policies;
        let failed = json(
            f.run("run-policies", &[&f.repo, &out, first, second, third], None),
            1,
        );
        assert_eq!(failed["diagnostics"].as_array().unwrap().len(), 6);
        assert_eq!(f.directory_memberships(), before);
    }
    let recovered = json(good(), 0);
    assert_eq!(recovered["diagnostics"].as_array().unwrap().len(), 6);
    assert_ne!(f.directory_memberships(), before);
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
