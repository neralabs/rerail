//! The slice's napi entry point against the slice itself (06 S6): the host's object finds what `audit` finds, and
//! a warm run over the same snapshot evaluates nothing. The engine's: only it compiles the napi object.

use crate::code_rule_dsl::rule_api::CAPABILITIES;
use crate::code_rule_dsl::slice::fixtures::*;

/// The napi entry point finds what the slice's own `audit` finds, and a run over the same
/// snapshot afterwards is warm: nothing evaluated, nothing re-run.
#[test]
fn the_napi_slice_audits_and_applies_as_the_slice_does() {
    let dir = temp("napi-dir");
    std::fs::create_dir_all(&dir).unwrap();
    write(&dir, "a.py", "assert x\nimport os\nos.chmod('f', 0o777)\n");
    write(&dir, "b.py", "def f():\n    password = 'hunter2'\n    return password\n");
    let mut documents = Vec::new();
    files(&pack().join("rules"), "json", &mut documents);
    files(&pack().join("candidates/rules"), "json", &mut documents);
    documents.sort();
    let sources = documents
        .iter()
        .map(|path| super::napi::SlicePolicy {
            name: path.file_stem().unwrap().to_string_lossy().into_owned(),
            source: std::fs::read_to_string(path).unwrap(),
            format: "json".to_owned(),
        })
        .filter(|policy| crate::code_rule_dsl::rule_api::compile(&policy.source, &policy.format, CAPABILITIES).is_ok())
        .collect::<Vec<_>>();
    let store = temp("napi-store").with_extension("sqlite");
    let mut slice = super::napi::CodeRuleSlice::new(store.to_string_lossy().into_owned(), sources, CAPABILITIES.to_owned(), "cpu".to_owned()).unwrap();
    assert!(slice.clauses() > 0);
    let found = |json: &str| -> (Keys, usize, u64) {
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let keys = value["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| (k["rule"].as_str().unwrap().to_owned(), k["code"].as_str().unwrap().to_owned(), k["path"].as_str().unwrap().to_owned(), k["line"].as_u64().unwrap() as usize))
            .collect();
        (keys, value["evaluated"].as_array().unwrap().len(), value["rerun"].as_u64().unwrap())
    };
    let (keys, evaluated, _) = found(&slice.audit(WS.to_owned(), dir.to_string_lossy().into_owned(), None, None).unwrap());
    assert_eq!(keys, self::keys(&cold(&dir, &policies())));
    assert!(!keys.is_empty() && evaluated == 2);
    let (warm, evaluated, rerun) = found(&slice.apply(WS.to_owned(), dir.to_string_lossy().into_owned(), None, None).unwrap());
    assert_eq!((warm, evaluated, rerun), (keys.clone(), 0, 0));
    // The Git observer: a clean checkout's `HEAD` is the directory's snapshot, so it is warm too.
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git").env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1")
            .args(["-c", "user.name=rerail", "-c", "user.email=rerail@example.invalid", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(status.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&status.stderr));
    };
    git(&["init", "-q"]);
    git(&["add", "a.py", "b.py"]);
    git(&["commit", "-q", "-m", "fixture"]);
    let (head, evaluated, rerun) = found(&slice.apply(WS.to_owned(), dir.to_string_lossy().into_owned(), Some("head".to_owned()), None).unwrap());
    assert_eq!((head, evaluated, rerun), (keys.clone(), 0, 0));
    assert!(slice.apply(WS.to_owned(), dir.to_string_lossy().into_owned(), Some("nope".to_owned()), None).is_err());
    // The buffers observer: an unsaved edit of `b.py` and an unsaved new file in a new directory
    // are found as if written, and nothing on disk changes.
    let buffers = || {
        vec![
            super::napi::SliceBuffer { path: "b.py".to_owned(), source: "def f():\n    assert x\n    password = 'hunter2'\n    return password\n".to_owned() },
            super::napi::SliceBuffer { path: "pkg/c.py".to_owned(), source: "import os\nos.chmod('g', 0o777)\n".to_owned() },
        ]
    };
    assert!(slice.apply(WS.to_owned(), dir.to_string_lossy().into_owned(), Some("head".to_owned()), Some(buffers())).is_err());
    let (buffered, evaluated, _) = found(&slice.apply(WS.to_owned(), dir.to_string_lossy().into_owned(), None, Some(buffers())).unwrap());
    assert_eq!(evaluated, 2);
    let written = temp("napi-written");
    std::fs::create_dir_all(written.join("pkg")).unwrap();
    std::fs::copy(dir.join("a.py"), written.join("a.py")).unwrap();
    for buffer in buffers() {
        std::fs::write(written.join(&buffer.path), &buffer.source).unwrap();
    }
    assert_eq!(buffered, self::keys(&cold(&written, &policies())));
    assert_ne!(buffered, keys);
    assert!(!dir.join("pkg").exists());
}
