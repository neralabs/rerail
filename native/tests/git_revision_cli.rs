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

/// Each decision row contains its detail and, when it repeats, a count such as "(x2)".
fn profile_decisions(directory: &Path, topic: &str) -> Vec<String> {
    let trace = fs::read_to_string(directory.join("profile.txt")).unwrap();
    let mut lines = trace.lines();
    if !lines.any(|line| line.strip_suffix(':') == Some(topic)) { return Vec::new(); }
    lines.map_while(|line| line.strip_prefix("  - ")).map(str::to_owned).collect()
}

impl Checkout {
    fn new() -> Self {
        let area = tempfile::tempdir().unwrap();
        let repo = area.path().join("checkout");
        let policy = area.path().join("identifiers.rerail");
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

    fn command(&self, name: &str, paths: &[&Path], scope: Option<&str>) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rerail-engine"));
        command
            .env("RERAIL_TAPE_CACHE", self.area.path().join("store.sqlite"))
            .arg(name)
            .args(paths);
        if let Some(scope) = scope {
            command.args(["--revision", "HEAD", "--analysis-scope", scope]);
        }
        command
    }

    fn run(&self, name: &str, paths: &[&Path], scope: Option<&str>) -> Output {
        self.command(name, paths, scope).output().unwrap()
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
    let program = f.area.path().join("export.rerail");
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
fn selected_documents_keep_each_git_placement_of_a_shared_tape() {
    let f = Checkout::new();
    fs::copy(f.repo.join("a.py"), f.repo.join("sub/b.py")).unwrap();
    fs::write(f.repo.join("sub/ignored.py"), "ignored = True\n").unwrap();
    fs::write(f.repo.join(".rerailignore"), "sub/ignored.py\n").unwrap();
    // Both files originally had the same size. An index stat match can hide the replacement on Windows.
    let blob = f.git(&["rev-parse", "HEAD:a.py"]);
    f.git(&["update-index", "--cacheinfo", "100644", &blob, "sub/b.py"]);
    f.git(&["add", "--", "sub/ignored.py", ".rerailignore"]);
    f.git(&["commit", "-qm", "two selected paths share one blob"]);
    assert_eq!(f.git(&["rev-parse", "HEAD:sub/b.py"]), blob);
    let tree = f.git(&["rev-parse", "HEAD^{tree}"]);
    fs::write(f.repo.join("a.py"), "edited = 'working tree'\n").unwrap();
    let example = include_str!("../../docs/rule-syntax/git.md").split("```rr\n")
        .map(|block| block.split("\n```").next().unwrap())
        .find(|program| program.lines().any(|line| line == ".output origin")).unwrap();
    let policy = f.area.path().join("origin.rerail");
    let out = f.area.path().join("origin-output");
    let query = |root: &Path, scope: &str| {
        fs::write(&policy, format!("{example}\n{scope}\n.output git_source_file\n.output git_source_view\n")).unwrap();
        let result = f.run("logic", &[&policy, root, &out], Some("origins"));
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        assert!(f.run("csv", &[&out], None).status.success());
        let rows = |name: &str| {
            let csv = fs::read_to_string(out.join(format!("{name}.csv"))).unwrap();
            let mut rows: Vec<Vec<String>> = csv.lines().map(|row| row.split('\t').map(str::to_owned).collect()).collect();
            rows.sort();
            rows
        };
        let views = rows("git_source_view");
        assert_eq!(views.len(), 1, "{views:?}");
        assert_eq!(views[0][2], tree);
        let files = rows("git_source_file");
        for row in &files { assert_eq!(row[0], views[0][0]); }
        let selected = files.into_iter().filter(|row| row[1].ends_with(".py")).map(|row| {
            assert_eq!(row[2], "33188");
            assert_eq!(row[3], blob);
            row[1].clone()
        }).collect::<Vec<_>>();
        let documents = rows("origin");
        for row in &documents {
            assert_eq!(row[1], blob);
            assert_eq!(row[2], "python");
            assert!(selected.contains(&row[0]));
        }
        (views[0].clone(), selected, documents)
    };
    let (view, files, both) = query(&f.repo, "");
    assert_eq!(view[3], "");
    assert_eq!(files, ["a.py", "sub/b.py"]);
    assert_eq!(both.iter().map(|r| r[0].as_str()).collect::<Vec<_>>(), ["a.py", "sub/b.py"]);
    assert_eq!(both[0][3], both[1][3], "equal content and grammar share a tape");
    assert_ne!(both[0][4], both[1][4], "each path places its own document");
    let (same_view, same_files, narrowed) = query(&f.repo, ".scope exclude \"sub/**\"");
    assert_eq!((same_view, same_files), (view.clone(), files));
    assert_eq!(narrowed.len(), 1);
    assert_eq!(narrowed[0][..4], both[0][..4]);
    let (subtree, files, nested) = query(&f.repo.join("sub"), "");
    assert_ne!(subtree[0], view[0]);
    assert_eq!(subtree[3], "sub");
    assert_eq!(files, ["b.py"]);
    assert_eq!(nested.len(), 1);
    assert_eq!(nested[0][0], "b.py");
    assert_eq!(nested[0][1..4], both[1][1..4]);

    // Exercise the raw filter reader with one valid key and two keys from outside the selected view.
    fs::write(&policy, r#"#include "git.rerail"
.decl git_inventory_selection(s: symbol)
.input git_inventory_selection
.decl git_inventory_child_request(r: symbol, s: symbol, directory: symbol, name: symbol, mode: number, encoding: symbol)
.decl git_inventory_child_of(r: symbol, s: symbol, directory: symbol, name: symbol, mode: number, encoding: symbol, path: symbol)
git_inventory_child_request(r, s, "", "a.py", 33188, "utf8") :- git_root_repo(r), git_inventory_selection(s).
git_inventory_child_request("another-repository", s, "", "a.py", 33188, "utf8") :- git_inventory_selection(s).
git_inventory_child_request(r, "another-selection", "", "a.py", 33188, "utf8") :- git_root_repo(r).
.decl chosen(repo: symbol, path: symbol)
chosen(r,p) :- git_inventory_child_request(r,s,d,n,m,c), git_inventory_child_of(r,s,d,n,m,c,p).
.output chosen
"#).unwrap();
    let run = f.run("logic", &[&policy, &f.repo, &out], Some("origins"));
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(f.run("csv", &[&out], None).status.success());
    let children = fs::read_to_string(out.join("chosen.csv")).unwrap();
    assert_eq!(children.lines().count(), 1, "{children}");
    assert!(children.starts_with(&view[1]) && children.trim_end().ends_with("\ta.py"), "{children}");
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
    let rule = f.area.path().join("settings.rerail");
    fs::write(&rule, r#"#include "python/calls.rerail"
.decl label(s: symbol)
label(s) :- file_line("a.py", _, s).
.decl found(s: symbol)
found(s) :- label(s), call_open(n).
.output found
"#).unwrap();
    let result = f.command("logic", &[&rule, &f.repo, &out], Some("constants")).env("RERAIL_THREADS", "2").output().unwrap();
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
    let broken = f.area.path().join("bad references.rerail");
    fs::write(&broken, "#include \"python/references.rerail\"\n.policy \"broken\" revision 1\n.language python\n.references {\"unknownParameter\":true}\n.violation Edge(at: node) \"edge\"\nEdge(n) :- reference_edge(n,_,_).\n").unwrap();
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
                    .contains("unknownParameter"),
                "{failed_run}"
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
    let policy = f.area.path().join("history.rerail");
    let out = f.area.path().join("rows");
    fs::write(&policy, "#include \"analysis.rerail\"\n.decl previous(r: symbol)\nprevious(r) :- analysis_scope_run(_,r).\n.output previous\n").unwrap();
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
fn requested_history_reuses_only_the_named_scopes_records() {
    use rerail_engine::content_store::{ContentStore, analysis_scope::{self, AnalysisRun, RunSource}};

    let f = Checkout::new();
    let database = f.area.path().join("store.sqlite");
    let profile = f.area.path().join("profile");
    let out = f.area.path().join("history");
    let policy = f.area.path().join("release.rerail");
    let tree = f.git(&["rev-parse", "HEAD^{tree}"]);
    let program = include_str!("../../docs/rule-syntax/git.md").split("```rr\n")
        .filter_map(|part| part.split_once("```")).map(|(program, _)| program)
        .find(|program| program.lines().any(|line| line == ".output release_scan")).unwrap();
    fs::write(&policy, program).unwrap();
    let publish = |scope: &str, scan: &str| {
        let mut run = AnalysisRun::begin(ContentStore::open(&database).unwrap(), scope, scan).unwrap();
        run.select_source(RunSource::Git { repository: "fixture".into(), tree: tree.clone(), commit: None }).unwrap();
        run.finish(true).unwrap();
    };
    let query = |ids: &[&str], hit: bool| {
        let result = f.command("logic", &[&policy, &f.repo, &out], Some("reader"))
            .env("RERAIL_PROFILE", &profile).output().unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        assert!(f.run("csv", &[&out], None).status.success());
        let csv = fs::read_to_string(out.join("release_scan.csv")).unwrap();
        let mut rows: Vec<_> = csv.lines().collect();
        rows.sort_unstable();
        assert_eq!(rows, ids.iter().map(|id| format!("{id}\t{tree}")).collect::<Vec<_>>());
        let decisions = fs::read_to_string(profile.join("profile.txt")).unwrap();
        assert_eq!(decisions.contains("scope history reuse"), hit, "{decisions}");
        let expected = format!("{:?}: {} run records", "release", ids.len());
        assert!(decisions.contains(&expected), "{decisions}");
    };
    publish("named:release", "first");
    query(&["first"], false);
    for i in 0..64 { publish("named:other", &format!("unrelated-{i}")); }
    assert_eq!(f.settled_store().query_row("SELECT count(*) FROM analysis_scope_run WHERE scope='named:other'", [], |r| r.get::<_, u32>(0)).unwrap(), 64);
    query(&["first"], true);
    analysis_scope::release_named_scope(&database, "other", &[]).unwrap();
    query(&["first"], true);
    publish("named:release", "second");
    query(&["first", "second"], false);
    analysis_scope::release_named_scope(&database, "release", &[]).unwrap();
    query(&[], false);
    query(&[], true);

    // A directory scope and a named scope can have the same display value. The store's opaque ID selects one.
    let store = ContentStore::open(&database).unwrap().identity();
    publish("cwd:/review", "directory");
    publish("named:/review", "named");
    let directory = format!("scope:{store}:cwd:/review");
    let foreign = format!("scope:{}:cwd:/review", store.wrapping_add(1));
    fs::write(&policy, format!(r#"#include "analysis.rerail"
analysis_history_request("scope", s) :- analysis_directory_scope("/review", s).
analysis_history_request("scope", "{foreign}").
.decl exact(key: symbol, id: symbol)
exact(key, id) :- analysis_history_of("scope", key, _, _, id, _, _, _, _, _).
.output exact
"#)).unwrap();
    let run = f.run("logic", &[&policy, &f.repo, &out], Some("reader"));
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(f.run("csv", &[&out], None).status.success());
    assert_eq!(fs::read_to_string(out.join("exact.csv")).unwrap().trim_end(), format!("{directory}\tdirectory"));
}

#[test]
fn revision_constant_records_follow_their_inputs_and_published_owners() {
    use rerail_engine::content_store::{self, ContentStore, CorpusPolicy};

    let f = Checkout::new();
    let marker = f.repo.join("marker");
    fs::write(&marker, "selected\n").unwrap();
    f.git(&["add", "marker"]);
    f.git(&["commit", "-qm", "marker"]);
    let first = f.git(&["rev-parse", "HEAD"]);
    let database = f.area.path().join("store.sqlite");
    let profile = f.area.path().join("profile");
    let policy = f.area.path().join("marker.rerail");
    fs::write(&policy, r#".policy "marker" revision 1
.language python
.violation Seen(at: node, s: symbol) "got {s}"
.decl marker(s: symbol)
marker(s) :- file_line("marker", _, s).
Seen(n, s) :- marker(s), kind(n, "identifier").
"#).unwrap();
    let scan = |revision: &str, scope: &str, value: &str, reused: bool| {
        let output = Command::new(env!("CARGO_BIN_EXE_rerail-engine"))
            .env("RERAIL_PROFILE", &profile).env("RERAIL_TAPE_CACHE", &database)
            .arg("run-rr").arg(&policy).arg(&f.repo).arg(f.area.path().join("out"))
            .args(["--revision", revision, "--analysis-scope", scope]).output().unwrap();
        let report = json(output, 0);
        let findings = report["diagnostics"].as_array().unwrap();
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().all(|row| row["parameters"]["s"] == value), "{findings:?}");
        let decisions = fs::read_to_string(profile.join("profile.txt")).unwrap();
        let marker = decisions.lines().find(|line| line.contains("given as facts: marker")).unwrap();
        assert_eq!(marker.contains("kept from an earlier run"), reused, "{marker}");
    };
    let versions = || {
        let observer = rusqlite::Connection::open_with_flags(content_store::observe::store_beside(&database), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        observer.query_row("SELECT count(*) FROM run_record WHERE key GLOB 'constants:*:*:*'", [], |r| r.get::<_, u32>(0)).unwrap()
    };
    scan(&first, "one", "selected", false);
    scan(&first, "one", "selected", true);
    scan(&first, "two", "selected", true);
    fs::write(f.repo.join("other"), "unrelated").unwrap();
    f.git(&["add", "other"]);
    f.git(&["commit", "-qm", "unrelated edit"]);
    scan("HEAD", "two", "selected", true);
    assert_eq!(versions(), 1);
    fs::write(&marker, "changed\n").unwrap();
    f.git(&["add", "marker"]);
    f.git(&["commit", "-qm", "changed marker"]);
    scan("HEAD", "one", "changed", false);
    scan(&first, "two", "selected", true);
    assert_eq!(versions(), 2);

    let oid = f.git(&["rev-parse", "HEAD:marker"]);
    let object = f.repo.join(".git/objects").join(&oid[..2]).join(&oid[2..]);
    let held = f.area.path().join("held-marker");
    fs::rename(&object, &held).unwrap();
    let unreadable = json(f.run("run-rr", &[&policy, &f.repo, &f.area.path().join("out")], Some("one")), 0);
    assert_eq!(unreadable["diagnostics"].as_array().unwrap().len(), 0);
    assert_eq!(versions(), 2);
    fs::rename(&held, &object).unwrap();
    scan("HEAD", "one", "changed", true);
    f.git(&["repack", "-ad"]);
    scan(&first, "two", "selected", true);
    scan("HEAD", "one", "changed", true);
    assert_eq!(versions(), 2);

    let release = |scope: &str| {
        let (id, members) = content_store::analysis_scope::release_named_scope(&database, scope, &[]).unwrap();
        assert!(members);
        let store = ContentStore::open_for_collection(&database, &[id]).unwrap();
        let now = content_store::unix_seconds().unwrap();
        let policy = CorpusPolicy::default();
        for _ in 0..64 {
            if !store.corpus_pass_wanted(&policy, now).unwrap() { return; }
            store.collect_corpus_pass(&policy, now).unwrap();
        }
        panic!("scope collection did not finish");
    };
    release("one");
    assert_eq!(versions(), 1);
    scan(&first, "two", "selected", true);
    release("two");
    assert_eq!(versions(), 0);
    scan(&first, "three", "selected", false);
}

#[test]
fn content_handles_read_and_retain_only_verified_object_bytes() {
    use rerail_engine::content_store::{self, ContentStore, CorpusPolicy, HashKey};
    use sha2::{Digest, Sha256};
    let f = Checkout::new();
    let payload = [0, 0xff, 0x13, 0x25, 0x41];
    fs::write(f.repo.join("example.bin"), payload).unwrap();
    f.git(&["add", "--", "example.bin"]);
    f.git(&["commit", "-qm", "binary"]);
    let blob = f.git(&["rev-parse", "HEAD:example.bin"]);
    let object = f.repo.join(".git/objects").join(&blob[..2]).join(&blob[2..]);
    let database = f.area.path().join("store.sqlite");
    let canonical = HashKey::sha256(&payload);
    let digest: [u8; 32] = Sha256::digest(payload).into();
    let handle = format!("sha256:{}", digest.iter().map(|b| format!("{b:02x}")).collect::<String>());
    let example = include_str!("../../docs/rule-syntax/git.md").split("```rr\n")
        .map(|block| block.split("\n```").next().unwrap())
        .find(|program| program.lines().any(|line| line == ".output signature")).unwrap();
    let policy = f.area.path().join("content.rerail");
    let out = f.area.path().join("content-output");
    fs::write(&policy, format!("{example}\n{}", r#"
git_content_read_request(h,3,100) :- git_blob_content(_,_,h).
git_content_read_request(h,9,2) :- git_blob_content(_,_,h).
git_content_read_request(h,-1,2) :- git_blob_content(_,_,h).
git_content_read_request("sha256:unacquired",0,1).
.decl acquired(blob: symbol, kind: symbol, handle: symbol, size: number)
acquired(b,k,h,n) :- git_object(_,b,k,h), git_content_size(h,n).
.decl errors(oid: symbol, reason: symbol)
errors(id,why) :- git_content_error(_,id,why).
.output acquired
.output errors
.output git_content_bytes
.output git_content_read_error
"#)).unwrap();
    let profile = f.area.path().join("content-profile");
    let scan = |scope: &str, failure: Option<&str>| {
        let output = f.command("logic", &[&policy, &f.repo, &out], Some(scope)).env("RERAIL_PROFILE", &profile).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(f.run("csv", &[&out], None).status.success());
        let read = |name: &str| fs::read_to_string(out.join(format!("{name}.csv"))).unwrap();
        if let Some(failure) = failure {
            assert_eq!(read("errors").trim_end(), format!("{blob}\t{failure}"));
            for name in ["acquired","signature","git_content_bytes"] { assert!(read(name).is_empty(), "{name}"); }
        } else {
            assert_eq!(read("acquired").trim_end(), format!("{blob}\tblob\t{handle}\t5"));
            assert_eq!(read("signature").trim_end(), "00ff1325");
            let bytes = read("git_content_bytes");
            let mut rows: Vec<_> = bytes.lines().collect();
            rows.sort();
            assert_eq!(rows, [format!("{handle}\t0\t4\t00ff1325"), format!("{handle}\t3\t100\t2541"), format!("{handle}\t9\t2\t")]);
            assert_eq!(read("git_content_read_error").trim_end(), format!("{handle}\t-1\t2\tinvalid-range"));
            assert!(read("errors").is_empty());
        }
        profile_decisions(&profile, "Git payload verified")
    };
    let stored = || ContentStore::open(&database).unwrap().source_by_hash(canonical).unwrap();
    assert_eq!(scan("first", None), [blob.as_str()]);
    let id = stored().expect("the ordinary source store holds the payload");
    assert_eq!(ContentStore::open(&database).unwrap().get_source(id).unwrap(), payload);
    fs::write(f.repo.join("a.py"), "another = 2\n").unwrap();
    f.git(&["add", "--", "a.py"]);
    f.git(&["commit", "-qm", "unrelated"]);
    assert!(scan("second", None).is_empty(), "an unrelated commit reuses the verified bytes");
    assert_eq!(stored(), Some(id));
    let release = |scope: &str| {
        let (store_id, members) = content_store::analysis_scope::release_named_scope(&database, scope, &[]).unwrap();
        assert!(members);
        let store = ContentStore::open_for_collection(&database, &[store_id]).unwrap();
        let now = content_store::unix_seconds().unwrap();
        let policy = CorpusPolicy::default();
        for _ in 0..64 {
            if !store.corpus_pass_wanted(&policy, now).unwrap() { break; }
            store.collect_corpus_pass(&policy, now).unwrap();
        }
        store.collect(now, 256).unwrap();
    };
    release("first");
    assert_eq!(stored(), Some(id));
    let offline = f.area.path().join("offline-blob");
    fs::rename(&object, &offline).unwrap();
    scan("second", Some("missing"));
    // Git can inflate the replacement as a blob, but its checksum belongs to another object.
    let wrong = f.git(&["rev-parse","HEAD:a.py"]);
    fs::copy(f.repo.join(".git/objects").join(&wrong[..2]).join(&wrong[2..]), &object).unwrap();
    scan("second", Some("unreadable"));
    fs::remove_file(&object).unwrap();
    fs::rename(offline, &object).unwrap();
    scan("second", None);
    f.git(&["repack", "-ad"]);
    scan("second", None);
    release("second");
    assert_eq!(stored(), None);
}

#[test]
fn content_handles_return_git_payloads_for_all_four_object_kinds() {
    use sha2::{Digest, Sha256};
    let f = Checkout::new();
    f.git(&["tag", "-a", "sample-tag", "-m", "an annotated tag"]);
    let policy = f.area.path().join("raw.rerail");
    let out = f.area.path().join("raw");
    let mut program = String::from(r#"#include "git/content.rerail"
.decl wanted(object: symbol)
git_content_request(r,id) :- git_root_repo(r), wanted(id).
git_content_read_request(h,0,n) :- git_content_size(h,n).
.decl checked(kind: symbol, id: symbol, h: symbol, bytes: symbol)
checked(k,id,h,bytes) :- git_object(_,id,k,h), git_content_bytes(h,0,_,bytes).
.output checked
.output git_content_error
"#);
    let mut expected = Vec::new();
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    for (kind, rev) in [("tree","HEAD^{tree}"),("commit","HEAD"),("blob","HEAD:a.py"),("tag","refs/tags/sample-tag")] {
        let id = f.git(&["rev-parse",rev]);
        let raw = Command::new("git").current_dir(&f.repo).args(["cat-file",kind,&id]).output().unwrap();
        assert!(raw.status.success());
        expected.push(format!("{kind}\t{id}\tsha256:{}\t{}", hex(&Sha256::digest(&raw.stdout)), hex(&raw.stdout)));
        program.push_str(&format!("wanted(\"{id}\").\n"));
    }
    program.push_str("git_content_request(\"unopened\",\"x\").\n");
    fs::write(&policy, program).unwrap();
    let run = f.run("logic", &[&policy, &f.repo, &out], Some("objects"));
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(f.run("csv", &[&out], None).status.success());
    let csv = fs::read_to_string(out.join("checked.csv")).unwrap();
    let mut rows: Vec<_> = csv.lines().collect();
    rows.sort();
    expected.sort();
    assert_eq!(rows, expected);
    assert_eq!(fs::read_to_string(out.join("git_content_error.csv")).unwrap().trim_end(), "unopened\tx\trepository-not-open");
}

#[test]
fn commit_locations_keep_the_bound_repository_commit_and_optional_field_in_reports() {
    let f = Checkout::new();
    let parent = f.git(&["rev-parse", "HEAD"]);
    fs::write(f.repo.join("git"), "file witness\n").unwrap();
    f.git(&["add", "--", "git"]);
    f.git(&["commit", "-qm", "WIP file named git"]);
    let child = f.git(&["rev-parse", "HEAD"]);
    let policy = f.area.path().join("places.rerail");
    let other = f.area.path().join("other.rerail");
    let out = f.area.path().join("report");
    let program = r#"#include "git.rerail"
.policy "places" revision 1
.violation Seen(at: loc, witness: symbol) "{witness}"
Seen(@git_commit_location(r,p,"message"), "parent") :- git_source_commit(r,c), git_parent(r,c,_,p).
Seen(@git_commit_location(r,c), "child") :- git_source_commit(r,c).
Seen(@place("git",1), "file") :- file_line("git",1,"file witness").
"#;
    fs::write(&policy, program).unwrap();
    fs::write(&other, program.replace("\"places\"", "\"other\"")).unwrap();
    for (name, paths, copies) in [
        ("run-rr", vec![policy.as_path(), &f.repo, &out], 1),
        ("run-policies", vec![f.repo.as_path(), &out, &policy, &other], 2),
    ] {
        let report = json(f.run(name, &paths, Some("locations")), 0);
        let rows = report["diagnostics"].as_array().unwrap();
        assert_eq!(rows.len(), 3 * copies, "{report}");
        for policy_name in ["places", "other"].into_iter().take(copies) {
            let find = |witness: &str| rows.iter().find(|r| r["policy"] == policy_name && r["parameters"]["witness"] == witness).unwrap();
            let file = find("file");
            assert_eq!(file["path"], "git");
            assert_eq!(file["line"], 1);
            assert!(file.get("location").is_none());
            let repository = &find("parent")["location"]["repository"];
            assert!(!repository.as_str().unwrap().is_empty());
            for (witness, id, field) in [("parent", &parent, Some("message")), ("child", &child, None)] {
                let row = find(witness);
                assert_eq!(row["location"], serde_json::json!({"kind":"git-commit","repository":repository,"commit":id,"field":field}));
                assert_eq!(row["message"], witness);
                for key in ["at", "path", "line"] { assert!(row[key].is_null(), "{row}"); }
            }
        }
    }
    let example = include_str!("../../docs/rule-syntax/git.md").split("```rr\n")
        .filter_map(|block| block.split_once("\n```").map(|(text, _)| text))
        .find(|program| program.contains(".violation WorkInProgress(")).unwrap();
    fs::write(&policy, example).unwrap();
    let report = json(f.run("run-rr", &[&policy, &f.repo, &out], Some("locations")), 0);
    let rows = report["diagnostics"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{report}");
    assert_eq!(rows[0]["location"]["commit"], child);
    assert_eq!(rows[0]["location"]["field"], "message");
}

#[test]
fn bound_commit_metadata_requests_each_key_and_guards_absence() {
    let f = Checkout::new();
    let root = f.git(&["rev-parse", "HEAD"]);
    f.git(&["commit", "--allow-empty", "-qm", "child"]);
    let child = f.git(&["rev-parse", "HEAD"]);
    let tree = f.git(&["rev-parse", "HEAD^{tree}"]);
    let missing = "1111111111111111111111111111111111111111";
    let policy = f.area.path().join("bound.rerail");
    let out = f.area.path().join("bound-results");
    let direct = format!(r#"#include "git.rerail"
.decl candidate(id: symbol)
candidate("{root}"). candidate("{child}"). candidate("{tree}"). candidate("{missing}").
.decl roots(id: symbol)
roots(id) :- git_root_repo(r), candidate(id), !git_parent(r, id, _, _).
.decl selected(id: symbol, message: symbol)
selected(id, msg) :- git_source_commit(r, id), git_subject(r, id, msg).
.decl parent_subject(id: symbol, subject: symbol)
parent_subject(p, subject) :- git_subject(r, p, subject), git_parent(r, c, _, p), git_source_commit(r, c).
.decl failed(id: symbol, reason: symbol)
failed(id, reason) :- git_object_error(_, id, reason).
.output roots, selected, parent_subject, failed, git_object_request
"#);
    for requests in ["", "git_object_request(r, id) :- git_root_repo(r), candidate(id)."] {
        fs::write(&policy, format!("{direct}\n{requests}\n")).unwrap();
        let result = f.run("logic", &[&policy, &f.repo, &out], Some("metadata"));
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        assert!(f.run("csv", &[&out], None).status.success());
        let read = |name: &str| fs::read_to_string(out.join(format!("{name}.csv"))).unwrap().trim_end().to_owned();
        assert_eq!(read("roots"), root);
        assert_eq!(read("selected"), format!("{child}\tchild"));
        assert_eq!(read("parent_subject"), format!("{root}\ttwo identifiers"));
        assert_eq!(read("failed"), format!("{missing}\tmissing"));
        let rows = read("git_object_request");
        let mut keys: Vec<_> = rows.lines().map(|row| row.split_once('\t').unwrap().1).collect();
        keys.sort();
        let mut expected = [&*root, &*child, &*tree, missing];
        expected.sort();
        assert_eq!(keys, expected);
    }
    // The join key selects the commit. Projecting it away too early would also ask for the missing object.
    fs::write(&policy, format!(r#"#include "git.rerail"
.decl batch(label: symbol, oid: symbol)
batch("keep", "{root}"). batch("drop", "{missing}").
.decl chosen(label: symbol)
chosen("keep").
.decl subject(s: symbol)
subject(s) :- git_root_repo(r), batch(label, c), chosen(label), git_subject(r, c, s).
.output subject, git_object_request
"#)).unwrap();
    let profile = f.area.path().join("profile-batch");
    let result = f.command("logic", &[&policy, &f.repo, &out], Some("metadata"))
        .env("RERAIL_TAPE_CACHE", f.area.path().join("batch.sqlite"))
        .env("RERAIL_PROFILE", &profile).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(profile_decisions(&profile, "Git metadata object read"), [root.as_str()]);
    assert!(f.run("csv", &[&out], None).status.success());
    assert_eq!(fs::read_to_string(out.join("subject.csv")).unwrap().trim_end(), "two identifiers");
    let requests = fs::read_to_string(out.join("git_object_request.csv")).unwrap();
    assert_eq!(requests.lines().map(|row| row.split_once('\t').unwrap().1).collect::<Vec<_>>(), [root]);
}

#[test]
fn ordered_commit_trailers_match_git_for_each_requested_message() {
    let f = Checkout::new();
    let message = f.area.path().join("message.txt");
    let policy = f.area.path().join("trailers.rerail");
    let out = f.area.path().join("trailers");
    let mut source = String::from(r#"#include "git.rerail"
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
    let rules = f.area.path().join("locations.rerail");
    let out = f.area.path().join("locations");
    fs::create_dir(f.area.path().join("includes")).unwrap();
    fs::write(f.area.path().join("includes/local.rerail"), "#include \"../common.rerail\"\n").unwrap();
    fs::write(f.area.path().join("common.rerail"), "#include \"git.rerail\"\n").unwrap();
    fs::write(&rules, r#"#include "includes/local.rerail"
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
    let policy = area.path().join("words.rerail");
    fs::write(&policy, "#include \"git.rerail\"\n.policy \"words\" revision 1\n.violation Seen(at: span) \"commit\"\nSeen(@place(\"git\", 1)) :- git_root_repo(_).\n").unwrap();
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
    let policy = f.area.path().join("head.rerail");
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

    let missing = format!("#include \"git.rerail\"\ngit_view_revision_request(\"{}\", \"HEAD\").\ngit_view_revision_request(v,\"no-such-ref\") :- git_root_view(_, v).\n.output git_view_revision_error\n", main[1]);
    let errors = evaluate(&linked, &missing, "git_view_revision_error");
    assert!(errors.lines().any(|row| row == format!("{}\tHEAD\trepository-not-open", main[1])), "{errors}");
    assert!(errors.lines().any(|row| row == format!("{}\tno-such-ref\tmissing-revision", link[1])), "{errors}");
}

#[test]
fn a_failed_working_tree_batch_preserves_the_previous_directory_memberships() {
    let f = Checkout::new();
    let out = f.area.path().join("batch");
    let other = f.area.path().join("other.rerail");
    fs::write(&other, ".policy \"other\" revision 1\n.language python\n.violation AlsoSeen(at: node) \"second identifier\"\nAlsoSeen(n) :- kind(n, \"identifier\").\n").unwrap();
    let bad = f.area.path().join("bad.rerail");
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

    let metadata = f.area.path().join("metadata.rerail");
    fs::write(&metadata, "#include \"git.rerail\"\n.policy \"meta\" revision 1\n.violation Commit(at: span) \"commit\"\nCommit(@place(\"git\", 1)) :- git_source_commit(_, _).\n").unwrap();
    let complete = json(
        f.run("run-rr", &[&metadata, &f.repo, &out], Some("metadata")),
        0,
    );
    assert_eq!(complete["status"], "completed");
    assert_eq!(complete["diagnostics"].as_array().unwrap().len(), 1);
    fs::write(&metadata, "#include \"git.rerail\"\n.policy \"meta\" revision 1\n.violation Commit(at: loc) \"commit\"\nCommit(@git_commit_location(r,c)) :- git_source_commit(r,c).\n").unwrap();
    let commit_only = json(f.run("run-rr", &[&metadata, &f.repo, &out], Some("metadata")), 0);
    assert_eq!(commit_only["status"], "completed");
    assert_eq!(commit_only["diagnostics"][0]["location"]["commit"], f.git(&["rev-parse", "HEAD"]));
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
