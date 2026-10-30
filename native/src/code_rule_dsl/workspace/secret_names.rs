//! Paths named like ones holding secrets, which the content store never holds a copy of.
//!
//! The host owns the names (`SENSITIVE_FILE_NAMES` and `SENSITIVE_FOLDER_NAMES` in
//! `src/engine/path-boundary.ts`) and passes them with the storage attach; this module only reads
//! them the way `sensitiveByName` there does, so the list has one source.

use crate::code_rule_dsl::json::invalid;
use napi::Result;
use napi_derive::napi;
use regex::Regex;

/// The names a host judges secret-like, as `enableStorageAsync` takes them. `files` are matched
/// against a path's last segment, case-insensitively, `*` standing for any run of characters and
/// `{a,b}` for either word; `folders` against every segment, compared lowercased.
#[napi(object)]
#[derive(Clone, Default)]
pub struct SecretNamePatterns {
    pub files: Vec<String>,
    pub folders: Vec<String>,
}

/// `SecretNamePatterns`, compiled.
#[derive(Default)]
pub(super) struct SecretNames {
    file: Option<Regex>,
    /// Lowercased.
    folders: Vec<String>,
}

impl SecretNames {
    pub(super) fn compile(patterns: &SecretNamePatterns) -> Result<Self> {
        let file = if patterns.files.is_empty() {
            None
        } else {
            let alternatives = patterns
                .files
                .iter()
                .map(|glob| glob_source(glob))
                .collect::<Vec<_>>()
                .join("|");
            Some(
                Regex::new(&format!("(?i)^(?:{alternatives})$"))
                    .map_err(|error| invalid(format!("invalid secret file name: {error}")))?,
            )
        };
        Ok(Self {
            file,
            folders: patterns
                .folders
                .iter()
                .map(|folder| folder.to_lowercase())
                .collect(),
        })
    }

    /// Whether the `/`-separated `path` under `prefix` (a path prefix, `""` or `dir/`) is named
    /// like one holding secrets: a folder on the way, in `prefix` or `path`, or its file.
    pub(super) fn matches(&self, prefix: &str, path: &str) -> bool {
        let mut segments = prefix
            .split('/')
            .chain(path.split('/'))
            .filter(|segment| !segment.is_empty());
        if segments.any(|segment| self.is_folder(segment)) {
            return true;
        }
        let name = path.rsplit('/').find(|segment| !segment.is_empty());
        match (&self.file, name) {
            (Some(file), Some(name)) => file.is_match(name),
            _ => false,
        }
    }

    fn is_folder(&self, segment: &str) -> bool {
        if segment.is_ascii() {
            self.folders
                .iter()
                .any(|folder| folder.eq_ignore_ascii_case(segment))
        } else {
            self.folders.contains(&segment.to_lowercase())
        }
    }
}

/// A file name glob as a regular expression's source, as `globSource` in `path-boundary.ts` reads
/// it: `*` is any run of characters, `{a,b}` either word, anything else itself.
fn glob_source(glob: &str) -> String {
    let mut source = String::new();
    let mut rest = glob;
    while let Some(first) = rest.chars().next() {
        if first == '*' {
            source.push_str(".*");
            rest = &rest[1..];
        } else if first == '{'
            && let Some(close) = rest[1..].find(['{', '}'])
            && rest.as_bytes()[1 + close] == b'}'
        {
            let choices = rest[1..1 + close]
                .split(',')
                .map(regex::escape)
                .collect::<Vec<_>>()
                .join("|");
            source.push_str(&format!("(?:{choices})"));
            rest = &rest[close + 2..];
        } else {
            let end = rest[1..]
                .find(['*', '{'])
                .map_or(rest.len(), |index| index + 1);
            source.push_str(&regex::escape(&rest[..end]));
            rest = &rest[end..];
        }
    }
    source
}

#[cfg(test)]
mod tests {
    use super::{SecretNamePatterns, SecretNames};

    /// The shapes `SENSITIVE_FILE_NAMES` uses; `path-boundary.test.ts` judges the same paths.
    fn names() -> SecretNames {
        let data = "{json,yaml,yml,xml,ini,toml,conf,cfg,properties,txt,csv,env}";
        SecretNames::compile(&SecretNamePatterns {
            files: [
                ".env",
                ".env.*",
                "id_rsa*",
                "*.pem",
                "credentials",
                &format!("credentials{{-,_}}*.{data}"),
                "secrets.*",
                &format!("*secret*.{data}"),
                ".*_history",
                "service-account*.json",
                "*-key.json",
                "kubeconfig.{json,yaml,yml}",
            ]
            .map(str::to_owned)
            .to_vec(),
            folders: [".ssh", ".aws"].map(str::to_owned).to_vec(),
        })
        .unwrap()
    }

    #[test]
    fn judges_paths_as_sensitive_by_name_does() {
        let names = names();
        for path in [
            ".env",
            "app/.env.local",
            "deploy/secrets.yaml",
            "SECRETS.YML",
            "keys/id_rsa.pub",
            "tls/server.pem",
            "credentials",
            "config/credentials-prod.json",
            "config/app-secrets.yaml",
            "config/client_secret.json",
            ".zsh_history",
            "ops/service-account-ci.json",
            "gcp-key.json",
            "kubeconfig.yaml",
            "home/.ssh/config",
            "home/.AWS/notes.txt",
        ] {
            assert!(names.matches("", path), "{path}");
        }
        for path in [
            "src/a.ts",
            "src/secret-manager.ts",
            "src/credentials-helper.ts",
            "keyboard.json",
            "kubeconfig.ts",
            "docs/env.md",
            ".github/workflows/ci.yml",
            "secrets",
        ] {
            assert!(!names.matches("", path), "{path}");
        }
    }

    #[test]
    fn a_folder_above_the_workspace_root_counts() {
        let names = names();
        assert!(names.matches("ops/.ssh/", "notes.txt"));
        assert!(!names.matches("ops/", "notes.txt"));
    }

    #[test]
    fn literal_glob_characters_stay_literal_and_no_names_match_nothing() {
        let literal = SecretNames::compile(&SecretNamePatterns {
            files: ["a.b", "{x", "c+d"].map(str::to_owned).to_vec(),
            folders: Vec::new(),
        })
        .unwrap();
        assert!(literal.matches("", "a.b") && !literal.matches("", "axb"));
        assert!(literal.matches("", "{x") && literal.matches("", "c+d"));
        assert!(!SecretNames::default().matches("", "secrets.yaml"));
    }
}
