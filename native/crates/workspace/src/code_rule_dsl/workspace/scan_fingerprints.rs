//! Fingerprints for the evaluated spans of a headless scan.

use super::{WorkspaceState, documents};
use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::violations::anchored_span;
use napi::Result;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeSet, HashMap};

type MatchKey = (String, usize, usize);

pub(super) struct ScanFingerprints {
    prefix: String,
    files: HashMap<String, FileFingerprints>,
}

struct FileFingerprints {
    version: documents::FileVersion,
    matches: HashMap<MatchKey, String>,
}

impl ScanFingerprints {
    pub(super) fn new(prefix: String) -> Self {
        Self { prefix, files: HashMap::new() }
    }
}

fn key(violation: &Value) -> Option<MatchKey> {
    let span = &violation["span"];
    Some((
        violation["ruleId"].as_str()?.to_owned(),
        usize::try_from(span["startByte"].as_u64()?).ok()?,
        usize::try_from(span["endByte"].as_u64()?).ok()?,
    ))
}

/// Capture local results before publication can release the evaluated source.
pub(super) fn capture_local(state: &mut WorkspaceState, paths: &BTreeSet<String>) -> Result<()> {
    if state.scan_fingerprints.is_none() {
        return Ok(());
    }
    for path in paths {
        let Some(file) = state.files.get(path) else { continue };
        let mut matches = Vec::new();
        for (&(rule, clause), witnesses) in file.witnesses.iter() {
            let rule = &state.code_rules[rule];
            let clause = &rule.clauses[clause];
            if clause.synthetic {
                continue;
            }
            for witness in witnesses {
                if let Some(violation) = &witness.sql_violation {
                    matches.extend(key(violation));
                } else if let (Some(span), _) = anchored_span(&clause.violation.anchor, witness) {
                    matches.push((rule.id.clone(), span.start_byte, span.end_byte));
                }
            }
        }
        capture(state, path, matches)?;
    }
    Ok(())
}

pub(super) fn capture_global(state: &mut WorkspaceState, violations: &[Value]) -> Result<()> {
    if state.scan_fingerprints.is_none() {
        return Ok(());
    }
    for violation in violations {
        if let (Some(path), Some(key)) = (violation["path"].as_str(), key(violation)) {
            capture(state, path, vec![key])?;
        }
    }
    Ok(())
}

fn capture(state: &mut WorkspaceState, path: &str, mut matches: Vec<MatchKey>) -> Result<()> {
    if matches.is_empty() {
        return Ok(());
    }
    let version = documents::current_version(state, path)
        .ok_or_else(|| invalid(format!("{path} has no evaluated source version")))?;
    if let Some(file) = state.scan_fingerprints.as_ref().and_then(|scan| scan.files.get(path))
        && file.version == version
    {
        matches.retain(|key| !file.matches.contains_key(key));
    }
    if matches.is_empty() {
        return Ok(());
    }
    let source = documents::source(state, path)?;
    let scan = state.scan_fingerprints.as_mut().expect("scan configured before capture");
    let report_path = if scan.prefix.is_empty() { path.to_owned() } else { format!("{}/{path}", scan.prefix) };
    let file = scan.files.entry(path.to_owned()).or_insert_with(|| FileFingerprints {
        version: version.clone(),
        matches: HashMap::new(),
    });
    if file.version != version {
        file.version = version;
        file.matches.clear();
    }
    for key in matches {
        let (rule, lo, hi) = &key;
        let text = source.get(*lo..*hi).ok_or_else(|| invalid(format!("{path} has an invalid evaluated span")))?;
        let fingerprint = fingerprint(rule, &report_path, text);
        file.matches.insert(key, fingerprint);
    }
    Ok(())
}

pub(super) fn of_violation(state: &mut WorkspaceState, violation: &Value) -> Result<Option<String>> {
    let (Some(path), Some(key), Some(_)) =
        (violation["path"].as_str(), key(violation), state.scan_fingerprints.as_ref())
    else {
        return Ok(None);
    };
    // Reused results can predate the scan. Their original source must still be available.
    capture(state, path, vec![key.clone()])?;
    Ok(state.scan_fingerprints.as_ref()
        .and_then(|scan| scan.files.get(path))
        .and_then(|file| file.matches.get(&key))
        .cloned())
}

fn fingerprint(rule: &str, path: &str, text: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(rule);
    hash.update([0]);
    hash.update(path);
    hash.update([0]);
    // Keep the same whitespace definition as the JSON scan report's JavaScript /\s/u.
    let whitespace = |ch| matches!(ch,
        '\t'..='\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}'
        | '\u{2028}'..='\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}');
    let mut words = text.split(whitespace).filter(|word| !word.is_empty());
    if let Some(word) = words.next() {
        hash.update(word);
        for word in words {
            hash.update(b" ");
            hash.update(word);
        }
    }
    let digest: [u8; 32] = hash.finalize().into();
    crate::content_store::observe::hex(&digest)[..32].to_owned()
}
