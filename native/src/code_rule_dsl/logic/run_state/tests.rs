//! The checks of [`super::STATE`] against the source, and of [`super::Carry`].

use super::{Carry, Class, NOT_YET_PINNED, STATE};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// `native/`.
fn native() -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .canonicalize()
        .unwrap()
}

fn rust_files(under: &Path, out: &mut Vec<PathBuf>) {
    let Ok(dir) = std::fs::read_dir(under) else { return };
    for e in dir.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// A file only tests compile.
fn test_file(p: &Path) -> bool {
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name == "tests.rs" || name.ends_with("_tests.rs") || p.components().any(|c| c.as_os_str() == "tests")
}

/// The names a `thread_local!` block declares.
fn statics(block: &str) -> Vec<String> {
    let b = block.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = block[at..].find("static ") {
        let s = at + i;
        at = s + 7;
        if s > 0 && !b[s - 1].is_ascii_whitespace() && b[s - 1] != b'{' {
            continue; // `'static `
        }
        let rest = block[at..].trim_start();
        let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if name.starts_with(|c: char| c.is_ascii_uppercase()) && rest[name.len()..].trim_start().starts_with(':') {
            out.push(name);
        }
    }
    out
}

/// Every thread-local outside the tests, by (file under `native/`, name).
fn declared() -> BTreeSet<(String, String)> {
    let root = native();
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    for c in std::fs::read_dir(root.join("crates")).unwrap().flatten() {
        rust_files(&c.path().join("src"), &mut files);
    }
    let mut out = BTreeSet::new();
    for p in files {
        if test_file(&p) {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap();
        let rel = p.strip_prefix(&root).unwrap().to_string_lossy().into_owned();
        let mut at = 0;
        while let Some(i) = text[at..].find("thread_local!") {
            let s = at + i;
            at = s + "thread_local!".len();
            if !text[at..].trim_start().starts_with('{') {
                continue; // named in a comment or a string
            }
            let open = s + text[s..].find('{').unwrap();
            let mut depth = 0usize;
            let mut end = open;
            for (j, ch) in text[open..].char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + j;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            at = end;
            let attr = text[..s].trim_end().lines().last().unwrap_or("").trim();
            if attr.starts_with("#[cfg") && attr.contains("test") {
                continue;
            }
            for n in statics(&text[open + 1..end]) {
                out.insert((rel.clone(), n));
            }
        }
    }
    out
}

#[test]
fn every_thread_local_is_listed_with_its_class() {
    let found = declared();
    let mut listed = BTreeSet::new();
    for (file, name, class, why) in STATE {
        assert!(listed.insert((file.to_string(), name.to_string())), "{file} {name} is listed twice");
        assert!(*class == Class::Carried || !why.is_empty(), "{file} {name}: say why it is {class:?}");
    }
    let unlisted: Vec<_> = found.difference(&listed).collect();
    assert!(
        unlisted.is_empty(),
        "thread-locals not in run_state::STATE (list each with its class: is it the run's context, which a job on \
         another thread must carry?): {unlisted:?}"
    );
    let gone: Vec<_> = listed.difference(&found).collect();
    assert!(gone.is_empty(), "listed in run_state::STATE but gone (cut them): {gone:?}");
}

#[test]
fn every_carried_thread_local_is_taken_by_a_carry() {
    let carried: BTreeSet<&str> = STATE.iter().filter(|s| s.2 == Class::Carried).map(|s| s.1).collect();
    let names = Carry::here().names();
    let taken: BTreeSet<&str> = names.iter().copied().collect();
    assert_eq!(names.len(), taken.len(), "a thread-local carried twice: {names:?}");
    assert_eq!(carried, taken, "STATE's Carried thread-locals and those `Carry::here` takes");
}

#[test]
fn the_context_not_carried_yet_only_shrinks() {
    let not_yet: Vec<_> = STATE.iter().filter(|s| s.2 == Class::NotYet).map(|s| (s.0, s.1)).collect();
    assert_eq!(
        not_yet.len(),
        NOT_YET_PINNED,
        "run context not carried yet: {not_yet:?}. Carry it (Class::Carried); when one is carried, lower the pin"
    );
}

#[test]
fn a_job_on_another_thread_enters_its_callers_context_and_leaves_its_own() {
    use crate::code_rule_dsl::phase_clock::IN_PHASE_0;
    IN_PHASE_0.with(|c| c.set(true));
    let carry = Carry::here();
    IN_PHASE_0.with(|c| c.set(false));
    let seen = std::thread::spawn(move || {
        IN_PHASE_0.with(|c| c.set(false));
        let inside = {
            let _in = carry.enter();
            IN_PHASE_0.with(|c| c.get())
        };
        (inside, IN_PHASE_0.with(|c| c.get()))
    })
    .join()
    .unwrap();
    assert_eq!(seen, (true, false), "inside the caller's context, then the thread's own again");
}

#[test]
fn a_pool_job_runs_in_the_context_it_was_started_in() {
    use crate::code_rule_dsl::phase_clock::IN_PHASE_0;
    let seen = std::sync::Mutex::new(Vec::new());
    IN_PHASE_0.with(|c| c.set(true));
    crate::threads::scope(|jobs| {
        for _ in 0..4 {
            jobs.spawn("run_state test", |_| seen.lock().unwrap().push(IN_PHASE_0.with(|c| c.get())));
        }
    });
    IN_PHASE_0.with(|c| c.set(false));
    let held = crate::threads::beside("run_state test", || IN_PHASE_0.with(|c| c.get()));
    assert!(!held.join().unwrap(), "started outside phase 0");
    assert_eq!(*seen.lock().unwrap(), vec![true; 4]);
}
