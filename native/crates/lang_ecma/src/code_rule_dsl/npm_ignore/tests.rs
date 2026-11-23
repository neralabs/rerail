use super::{NodeIgnore, NpmPublication};
use serde_json::Value;

const CASES: &str = include_str!("cases.json");

/// The answers a case records for each of `paths`; `None` where upstream throws.
fn answers(recorded: &Value) -> Vec<Option<bool>> {
    recorded
        .as_str()
        .expect("answers")
        .chars()
        .map(|answer| match answer {
            '1' => Some(true),
            '0' => Some(false),
            _ => None,
        })
        .collect()
}

/// node-ignore's own answers to `ignores(path)`, for each pattern set and path of the table.
#[test]
fn matches_node_ignore() {
    let cases: Value = serde_json::from_str(CASES).unwrap();
    let paths = cases["paths"].as_array().unwrap();
    let mut mismatches = Vec::new();
    for case in cases["ignore"].as_array().unwrap() {
        let patterns = case[0].as_str().unwrap();
        let mut ignore = NodeIgnore::default();
        ignore.add(patterns);
        for (path, expected) in paths.iter().zip(answers(&case[1])) {
            let path = path.as_str().unwrap();
            if let Some(expected) = expected
                && ignore.ignores(path) != expected
            {
                mismatches.push(format!(
                    "{patterns:?} ignores {path:?}: expected {expected}"
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// eslint-plugin-n's own answers to `getNpmignore(...).match(path)`, for each package and path of
/// the table.
#[test]
fn matches_eslint_plugin_ns_npmignore() {
    let cases: Value = serde_json::from_str(CASES).unwrap();
    let paths = cases["paths"].as_array().unwrap();
    let mut mismatches = Vec::new();
    for case in cases["publication"].as_array().unwrap() {
        let package = &case[0];
        let files = package["files"].as_array().map(|files| {
            files
                .iter()
                .filter_map(Value::as_str)
                .filter(|file| !file.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        });
        let publication = NpmPublication::new(
            "",
            files.as_deref(),
            package["main"].as_str(),
            package["npmignore"].as_str(),
            package["gitignore"].as_str(),
        );
        for (path, expected) in paths.iter().zip(answers(&case[1])) {
            let path = path.as_str().unwrap();
            if let Some(expected) = expected
                && publication.leaves_out(path) != expected
            {
                mismatches.push(format!(
                    "{package} leaves out {path:?}: expected {expected}"
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
