//! A compiled code rule's placement in one checkout: per folder, the mode it runs at there, or off,
//! and whether linters' suppression comments silence it there. A workspace holds it on its resident
//! copy of the code rule, so one compiled code rule can run in several folders of a checkout at
//! different modes without being compiled again.

use super::json::{invalid, json_error};
use super::model::{Placement, PlacementEntry, RuleMode};
use serde_json::Value;

/// The placement in `json`: an array of `{"folder": string, "mode": "log"|"steer"|"block"|null,
/// "suppressions"?: "honour"|"ignore"}`. A missing `suppressions` is `honour`.
/// Folders are normalised as the host normalises them (`\` is `/`, empty and `.` segments drop);
/// an absolute folder, a `..` segment or a folder named twice is rejected.
pub(super) fn parse_placement(json: &str) -> napi::Result<Placement> {
    let value: Value = serde_json::from_str(json).map_err(json_error)?;
    let entries = value
        .as_array()
        .ok_or_else(|| invalid("placement must be an array"))?;
    let mut placement = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| parse_entry(entry, &format!("placement/{index}")))
        .collect::<napi::Result<Vec<_>>>()?;
    placement.sort_by(|a, b| a.folder.cmp(&b.folder));
    if let Some(pair) = placement
        .windows(2)
        .find(|pair| pair[0].folder == pair[1].folder)
    {
        return Err(invalid(format!(
            "placement names folder `{}` more than once",
            pair[0].folder
        )));
    }
    Ok(placement.into())
}

fn parse_entry(entry: &Value, at: &str) -> napi::Result<PlacementEntry> {
    let object = entry
        .as_object()
        .ok_or_else(|| invalid(format!("{at} must be an object")))?;
    if let Some(key) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "folder" | "mode" | "suppressions"))
    {
        return Err(invalid(format!("{at}/{key} is not a placement field")));
    }
    let folder = object
        .get("folder")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("{at}/folder must be a string")))?;
    let mode = match object.get("mode") {
        Some(Value::Null) => Some(None),
        Some(Value::String(mode)) => RuleMode::parse(mode).map(Some),
        _ => None,
    }
    .ok_or_else(|| invalid(format!("{at}/mode must equal log, steer, block, or null")))?;
    let honour_suppressions = match object.get("suppressions") {
        None => Some(true),
        Some(Value::String(choice)) if choice == "honour" => Some(true),
        Some(Value::String(choice)) if choice == "ignore" => Some(false),
        _ => None,
    }
    .ok_or_else(|| invalid(format!("{at}/suppressions must equal honour or ignore")))?;
    Ok(PlacementEntry {
        folder: normalize_folder(folder, at)?,
        mode,
        honour_suppressions,
    })
}

fn normalize_folder(folder: &str, at: &str) -> napi::Result<String> {
    let posix = folder.replace('\\', "/");
    let posix = posix.trim();
    let drive = posix.as_bytes();
    if posix.starts_with('/')
        || (drive.len() >= 3 && drive[0].is_ascii_alphabetic() && &drive[1..3] == b":/")
    {
        return Err(invalid(format!(
            "{at}/folder must be relative to the checkout: {folder}"
        )));
    }
    let segments = posix
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect::<Vec<_>>();
    if segments.contains(&"..") {
        return Err(invalid(format!(
            "{at}/folder must stay inside the checkout: {folder}"
        )));
    }
    Ok(segments.join("/"))
}

/// The bytes a placement contributes to a result catalog's identity.
pub(super) fn placement_identity(placement: &[PlacementEntry]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for entry in placement {
        bytes.extend_from_slice(entry.folder.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(entry.mode.map_or("off", RuleMode::as_str).as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(if entry.honour_suppressions {
            b"honour"
        } else {
            b"ignore"
        });
        bytes.push(0);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::model::placed;

    fn modes(json: &str, paths: &[&str]) -> Vec<Option<Option<RuleMode>>> {
        let placement = parse_placement(json).unwrap();
        paths
            .iter()
            .map(|path| placed(&placement, path).map(|entry| entry.mode))
            .collect()
    }

    #[test]
    fn the_deepest_containing_folder_decides() {
        let json = r#"[
            {"folder": "web/legacy/keep", "mode": "log"},
            {"folder": "web", "mode": "steer"},
            {"folder": "web/legacy", "mode": null}
        ]"#;
        assert_eq!(
            modes(
                json,
                &[
                    "web/a.ts",
                    "web",
                    "web/legacy/a.ts",
                    "web/legacy/keep/a.ts",
                    "web2/x.ts",
                    "api/a.py",
                ]
            ),
            [
                Some(Some(RuleMode::Steer)),
                Some(Some(RuleMode::Steer)),
                Some(None),
                Some(Some(RuleMode::Log)),
                None,
                None,
            ]
        );
    }

    #[test]
    fn the_whole_checkout_folder_contains_every_path() {
        assert_eq!(
            modes(
                r#"[{"folder": "", "mode": "block"}, {"folder": "api", "mode": "log"}]"#,
                &["web/a.ts", "api/a.py"]
            ),
            [Some(Some(RuleMode::Block)), Some(Some(RuleMode::Log))]
        );
    }

    #[test]
    fn folders_are_normalised_before_duplicates_are_found() {
        let placement = parse_placement(r#"[{"folder": "./web//ui/", "mode": "log"}]"#).unwrap();
        assert_eq!(placement[0].folder, "web/ui");
        let error = parse_placement(
            r#"[{"folder": "web/", "mode": "log"}, {"folder": "web", "mode": null}]"#,
        )
        .unwrap_err();
        assert!(error.reason.contains("more than once"), "{}", error.reason);
    }

    #[test]
    fn invalid_placements_are_rejected() {
        for (json, reason) in [
            ("not json", ""),
            (r#"{"folder": "", "mode": "log"}"#, "must be an array"),
            (r#"[{"folder": "", "mode": "warn"}]"#, "/mode must equal"),
            (r#"[{"folder": ""}]"#, "/mode must equal"),
            (r#"[{"mode": "log"}]"#, "/folder must be a string"),
            (
                r#"[{"folder": "/etc", "mode": "log"}]"#,
                "relative to the checkout",
            ),
            (
                r#"[{"folder": "C:/web", "mode": "log"}]"#,
                "relative to the checkout",
            ),
            (
                r#"[{"folder": "web/../..", "mode": "log"}]"#,
                "inside the checkout",
            ),
            (
                r#"[{"folder": "", "mode": "log", "on": true}]"#,
                "not a placement field",
            ),
            (
                r#"[{"folder": "", "mode": "log", "suppressions": "skip"}]"#,
                "/suppressions must equal honour or ignore",
            ),
            (
                r#"[{"folder": "", "mode": "log", "suppressions": null}]"#,
                "/suppressions must equal honour or ignore",
            ),
            (
                r#"[{"folder": "", "mode": "log", "suppressions": false}]"#,
                "/suppressions must equal honour or ignore",
            ),
        ] {
            let error = parse_placement(json).expect_err(json);
            assert!(error.reason.contains(reason), "{json}: {}", error.reason);
        }
    }

    #[test]
    fn suppressions_default_to_honour_and_follow_the_deciding_folder() {
        let placement = parse_placement(
            r#"[
                {"folder": "", "mode": "log"},
                {"folder": "web", "mode": "log", "suppressions": "ignore"},
                {"folder": "web/vendor", "mode": "log", "suppressions": "honour"}
            ]"#,
        )
        .unwrap();
        let honours = |path| placed(&placement, path).map(|entry| entry.honour_suppressions);
        assert_eq!(
            ["api/a.py", "web/a.ts", "web/vendor/a.ts"].map(honours),
            [Some(true), Some(false), Some(true)]
        );
    }

    #[test]
    fn the_identity_differs_when_only_suppressions_differ() {
        let identity = |json| placement_identity(&parse_placement(json).unwrap());
        let omitted = identity(r#"[{"folder": "web", "mode": "log"}]"#);
        let honour = identity(r#"[{"folder": "web", "mode": "log", "suppressions": "honour"}]"#);
        let ignore = identity(r#"[{"folder": "web", "mode": "log", "suppressions": "ignore"}]"#);
        assert_eq!(omitted, honour);
        assert_ne!(honour, ignore);
    }
}
