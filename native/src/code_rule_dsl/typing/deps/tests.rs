use super::*;
use crate::code_rule_dsl::typing::terms::Terms;

const RECORD: &str = "vendor/__init__.py,sha256=AAA,10
vendor/core.py,sha256=BBB,200
vendor/core.pyi,sha256=CCC,50
vendor-1.2.dist-info/RECORD,,
";

const LOCK: &str = r#"
[[package]]
name = "vendor"
version = "1.3"

[[package]]
name = "other_pkg"
version = "0.1"
"#;

fn site(dir: &Path) {
    std::fs::create_dir_all(dir.join("vendor-1.2.dist-info")).unwrap();
    std::fs::write(dir.join("vendor-1.2.dist-info/RECORD"), RECORD).unwrap();
}

fn names() -> impl FnMut(&str) -> Sym {
    let mut v: Vec<String> = Vec::new();
    move |s: &str| match v.iter().position(|x| x == s) {
        Some(i) => i as Sym,
        None => {
            v.push(s.to_owned());
            (v.len() - 1) as Sym
        }
    }
}

#[test]
fn identity_comes_from_record_and_the_lockfile() {
    let dir = std::env::temp_dir().join(format!("rerail-deps-id-{}", std::process::id()));
    site(&dir);
    let lock = lock_versions(LOCK);
    assert_eq!(lock.get("other-pkg").map(String::as_str), Some("0.1"), "names are normalized");
    let installed = Installed::scan(&dir, Some(&lock)).unwrap();
    assert_eq!(installed.modules.keys().map(String::as_str).collect::<Vec<_>>(), ["vendor", "vendor.core"]);
    assert_eq!(installed.modules["vendor.core"].path, "vendor/core.pyi", "the stub is preferred");
    assert_eq!(installed.modules["vendor.core"].digest, "sha256=CCC");
    assert_eq!(installed.mismatches, vec![("vendor".to_owned(), "1.3".to_owned(), "1.2".to_owned())]);
    // The content address changes with the file, not with where it is installed.
    let a = installed.modules["vendor"].clone();
    let b = Identity { digest: "sha256=ZZZ".into(), ..a.clone() };
    assert_eq!(a.key(), a.clone().key());
    assert_ne!(a.key(), b.key());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn demand_chases_reexports_and_keeps_pending_apart_from_top() {
    let dir = std::env::temp_dir().join(format!("rerail-deps-demand-{}", std::process::id()));
    site(&dir);
    let installed = Installed::scan(&dir, None).unwrap();
    let cache = Cache { dir: dir.join("cache") };
    cache
        .put(&installed.modules["vendor"], "# rerail.types/v1\n# module: vendor\nfrom .core import Client as Client\ndef version() -> str: ...\n")
        .unwrap();
    // vendor.core has no summary yet: pending.
    let (mut terms, mut intern) = (Terms::default(), names());
    let u = Universe::demand(&["vendor", "missing"], &installed, &cache, 8, &mut terms, &mut intern);
    assert!(matches!(u.modules["vendor"], State::Ready(ref d) if d.signatures.contains_key("vendor.version")));
    assert!(matches!(u.modules["vendor.core"], State::Pending), "demanded through the re-export");
    assert!(matches!(u.modules["missing"], State::Top));
    assert_eq!(u.requested.iter().map(|i| i.path.as_str()).collect::<Vec<_>>(), ["vendor/core.pyi"]);
    assert!(u.reexports.contains(&("vendor".to_owned(), "Client".to_owned(), "vendor.core".to_owned())));
    // Once summarized, it is read.
    cache.put(&installed.modules["vendor.core"], "# rerail.types/v1\n# module: vendor.core\nclass Client:\n    def get(self) -> bytes: ...\n").unwrap();
    let u = Universe::demand(&["vendor"], &installed, &cache, 8, &mut terms, &mut intern);
    assert!(matches!(u.modules["vendor.core"], State::Ready(ref d) if d.signatures.contains_key("vendor.core.Client.get")));
    // The budget per package: past it, ⊤.
    let u = Universe::demand(&["vendor"], &installed, &cache, 1, &mut terms, &mut intern);
    assert!(matches!(u.modules["vendor.core"], State::Top));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn relative_reexports_resolve_against_the_module() {
    assert_eq!(reexports("a.b", false, "from .c import x, y as z"), vec![("x".into(), "a.c".into()), ("y".into(), "a.c".into())]);
    assert_eq!(reexports("a.b.c", false, "from .. import q"), vec![("q".into(), "a".into())]);
    assert_eq!(reexports("a", true, "from .c import x"), vec![("x".into(), "a.c".into())], "a package's own");
    assert_eq!(reexports("a", true, "from os.path import join"), vec![("join".into(), "os.path".into())]);
}
