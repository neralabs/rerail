//! Build identities shared by `build.rs` and the library's tests: the build script computes them,
//! and `cargo test` checks them without a separately compiled crate.

use sha2::{Digest, Sha256};

/// The identity of the parser and grammar versions a stored tape was parsed with, and of the
/// named sources that also decide a tape (a grammar built from vendored sources, how a tape's view
/// of its file is built).
pub fn grammar_build_id(lock_text: &str, sources: &[(&str, &[u8])]) -> String {
    let mut grammar = Sha256::new();
    grammar.update(b"rerail-grammar-build-v1\0");
    for package in grammar_packages(lock_text) {
        grammar.update(package.as_bytes());
        grammar.update([0]);
        grammar.update(locked_version(lock_text, &package).as_bytes());
        grammar.update([0]);
    }
    for (name, bytes) in sources {
        grammar.update(name.as_bytes());
        grammar.update([0]);
        grammar.update(bytes);
        grammar.update([0]);
    }
    hex(&grammar.finalize())
}

fn grammar_packages(lock: &str) -> Vec<String> {
    let mut packages = vec![
        "ast-grep-core".to_owned(),
        "ast-grep-language".to_owned(),
        "tree-sitter".to_owned(),
    ];
    // Cargo.lock is the resolved ast-grep-language feature graph. Include every locked
    // tree-sitter grammar so adding a parser feature cannot leave stale tapes reusable.
    packages.extend(
        lock.lines()
            .filter_map(|line| line.strip_prefix("name = \""))
            .filter_map(|line| line.strip_suffix('"'))
            .filter(|package| package.starts_with("tree-sitter-") && *package != "tree-sitter")
            .map(str::to_owned),
    );
    packages.sort();
    packages.dedup();
    packages
}

/// The locked version of one package; a missing package fails the build rather than silently
/// dropping out of the identity.
fn locked_version(lock: &str, package: &str) -> String {
    let header = format!("name = \"{package}\"\n");
    let start = lock
        .find(&header)
        .unwrap_or_else(|| panic!("{package} is not in Cargo.lock"));
    let rest = &lock[start + header.len()..];
    rest.lines()
        .next()
        .and_then(|line| line.strip_prefix("version = \""))
        .and_then(|line| line.strip_suffix('"'))
        .unwrap_or_else(|| panic!("{package} has no locked version"))
        .to_owned()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::grammar_build_id;

    const LOCK: &str = "[[package]]\nname = \"ast-grep-core\"\nversion = \"0.45.3\"\n\n\
        [[package]]\nname = \"ast-grep-language\"\nversion = \"0.45.3\"\n\n\
        [[package]]\nname = \"tree-sitter\"\nversion = \"0.27.0\"\n\n\
        [[package]]\nname = \"tree-sitter-javascript\"\nversion = \"0.25.0\"\n\n\
        [[package]]\nname = \"tree-sitter-php\"\nversion = \"0.24.0\"\n";

    #[test]
    fn grammar_build_id_changes_with_javascript_and_php_versions() {
        let baseline = grammar_build_id(LOCK, &[]);
        for (from, to) in [
            (
                "tree-sitter-javascript\"\nversion = \"0.25.0",
                "tree-sitter-javascript\"\nversion = \"0.25.1",
            ),
            (
                "tree-sitter-php\"\nversion = \"0.24.0",
                "tree-sitter-php\"\nversion = \"0.24.1",
            ),
        ] {
            assert_ne!(
                baseline,
                grammar_build_id(&LOCK.replace(from, to), &[]),
                "{to}"
            );
        }
        assert_eq!(baseline, grammar_build_id(LOCK, &[]));
    }
}
