//! The daemon talks to the tray in JSON lines: menu states arrive on stdin, clicks go back on stdout.

use std::io::{BufRead, Write};

use serde::Deserialize;

/// What the menu shows. The daemon sends one at start and another whenever anything in it changes.
#[derive(Deserialize, Clone, PartialEq, Debug)]
pub struct MenuState {
    pub healthy: bool,
    pub status: String,
    pub version: String,
    /// The third line, `Home path: <path>`. Its key is `dashboard`, the one the line had while it named the dashboard's
    /// address, so a tray and a daemon from either side of that change still read each other's lines.
    #[serde(rename = "dashboard")]
    pub home_path: String,
    pub update: Option<Item>,
    pub attention: bool,
    /// The Repositories section; None leaves it out, as from a daemon that sends none or has not read them yet.
    #[serde(default)]
    pub repositories: Option<Repositories>,
}

/// A line whose text the daemon gives, and whether it can be clicked.
#[derive(Deserialize, Clone, PartialEq, Debug)]
pub struct Item {
    pub label: String,
    pub enabled: bool,
}

/// The menu's Repositories section: a row per repository, and a last line under them, or none.
#[derive(Deserialize, Clone, PartialEq, Debug)]
pub struct Repositories {
    pub rows: Vec<RepositoryRow>,
    /// "3 more…", which opens the dashboard's list, or "No repositories yet", dimmed.
    pub more: Option<Item>,
}

/// A repository's row: a click sends its `key` back. `running` draws its circle green, else grey.
#[derive(Deserialize, Clone, PartialEq, Debug)]
pub struct RepositoryRow {
    pub key: String,
    pub label: String,
    pub running: bool,
}

/// A menu click the daemon acts on.
#[derive(Clone, PartialEq, Debug)]
pub enum Action {
    /// Go to the Dashboard: the dashboard's main page.
    Open,
    /// Settings…: the dashboard's Settings page.
    Settings,
    /// Documentation: Rerail's documentation site.
    Docs,
    /// A repository's row: the dashboard at the repository with this key.
    Repository(String),
    /// The line under the rows: the dashboard's list of every repository.
    Repositories,
    Update,
    Restart,
    Stop,
}

/// One JSON line, newline included, ready to write to stdout.
pub fn encode_action(action: &Action) -> String {
    let name = match action {
        Action::Open => "open",
        Action::Settings => "settings",
        Action::Docs => "docs",
        Action::Repository(key) => {
            // A JSON string as serde writes it, quotes and escapes included; a `String` always serializes.
            let key = serde_json::to_string(key).unwrap_or_default();
            return format!("{{\"action\":\"repository\",\"repository\":{key}}}\n");
        }
        Action::Repositories => "repositories",
        Action::Update => "update",
        Action::Restart => "restart",
        Action::Stop => "stop",
    };
    format!("{{\"action\":\"{name}\"}}\n")
}

/// Tells the daemon about a click. A failure is noted on stderr: the daemon is gone, or the click is lost.
pub fn write_action(action: &Action) {
    let mut stdout = std::io::stdout().lock();
    let written = stdout
        .write_all(encode_action(action).as_bytes())
        .and_then(|()| stdout.flush());
    if let Err(error) = written {
        eprintln!("rerail-tray: could not send {action:?} to the daemon: {error}");
    }
}

#[derive(PartialEq, Debug)]
pub enum Incoming {
    State(MenuState),
    /// Stdin ended or failed: the daemon is gone, so the tray exits.
    Closed,
}

/// Sends every valid line as a state, skips an invalid one (not UTF-8, or not a state) with a note on `diagnostics`,
/// and sends `Closed` once at end of input or on a read error, which also gets a note. The tray passes stderr, which
/// lands in `daemon.log`.
pub fn read_states(
    mut input: impl BufRead,
    mut diagnostics: impl Write,
    mut send: impl FnMut(Incoming),
) {
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        match input.read_until(b'\n', &mut bytes) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => {
                // A note that cannot be written has nowhere else to go.
                let _ = writeln!(
                    diagnostics,
                    "rerail-tray: stopped reading menu states: {error}"
                );
                break;
            }
        }
        // Read as bytes, not `lines()`, whose error for a line that is not UTF-8 would end the input.
        let line = match std::str::from_utf8(&bytes) {
            Ok(line) => line,
            Err(error) => {
                let _ = writeln!(
                    diagnostics,
                    "rerail-tray: skipped a menu state that is not UTF-8: {error}"
                );
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<MenuState>(line) {
            Ok(state) => send(Incoming::State(state)),
            Err(error) => {
                let _ = writeln!(
                    diagnostics,
                    "rerail-tray: skipped a menu state that did not parse: {error}"
                );
            }
        }
    }
    send(Incoming::Closed);
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{"healthy":true,"status":"Rerail is running","version":"Version 0.2.2","dashboard":"Home path: ~/.rerail","update":{"label":"Update to 0.2.3","enabled":true},"attention":true}"#;

    /// What `read_states` sends for `input`, and the notes it writes.
    fn collect(input: impl BufRead) -> (Vec<Incoming>, String) {
        let (mut seen, mut notes) = (Vec::new(), Vec::new());
        read_states(input, &mut notes, |incoming| seen.push(incoming));
        (seen, String::from_utf8(notes).unwrap())
    }

    #[test]
    fn parses_full_state() {
        let state: MenuState = serde_json::from_str(FULL).unwrap();
        assert_eq!(
            state,
            MenuState {
                healthy: true,
                status: "Rerail is running".into(),
                version: "Version 0.2.2".into(),
                home_path: "Home path: ~/.rerail".into(),
                update: Some(Item {
                    label: "Update to 0.2.3".into(),
                    enabled: true
                }),
                attention: true,
                repositories: None,
            }
        );
    }

    #[test]
    fn parses_state_without_update() {
        let line = FULL.replace(r#"{"label":"Update to 0.2.3","enabled":true}"#, "null");
        let state: MenuState = serde_json::from_str(&line).unwrap();
        assert_eq!(state.update, None);
    }

    #[test]
    fn reads_a_state_from_a_daemon_that_still_sends_home() {
        let line = FULL.replacen(
            r#""update""#,
            r#""home":"Home: /tmp/x/.rerail","update""#,
            1,
        );
        assert_eq!(
            serde_json::from_str::<MenuState>(&line).unwrap(),
            serde_json::from_str::<MenuState>(FULL).unwrap()
        );
    }

    #[test]
    fn a_tray_that_still_reads_home_takes_a_state_without_it() {
        // The shape of MenuState while it had the Home line: its `home` is an Option, which serde fills with None when
        // the key is absent.
        #[derive(Deserialize)]
        struct WithHome {
            home: Option<String>,
        }
        assert_eq!(serde_json::from_str::<WithHome>(FULL).unwrap().home, None);
    }

    #[test]
    fn reads_a_state_from_a_daemon_that_still_sends_the_dashboard_line() {
        let line = FULL.replace("Home path: ~/.rerail", "Dashboard at 127.0.0.1:4737");
        let state: MenuState = serde_json::from_str(&line).unwrap();
        assert_eq!(state.home_path, "Dashboard at 127.0.0.1:4737");
    }

    #[test]
    fn a_tray_that_still_shows_the_dashboard_line_takes_the_home_path_in_it() {
        // The shape of MenuState while its third line named the dashboard's address.
        #[derive(Deserialize)]
        struct WithDashboard {
            dashboard: String,
        }
        let state = serde_json::from_str::<WithDashboard>(FULL).unwrap();
        assert_eq!(state.dashboard, "Home path: ~/.rerail");
    }

    #[test]
    fn parses_the_repositories_section() {
        let line = FULL.replacen(
            r#""attention":true"#,
            r#""attention":true,"repositories":{"rows":[{"key":"remote:github.com/acme/web","label":"web · 3 rules on","running":true}],"more":{"label":"3 more…","enabled":true}}"#,
            1,
        );
        let state: MenuState = serde_json::from_str(&line).unwrap();
        assert_eq!(
            state.repositories,
            Some(Repositories {
                rows: vec![RepositoryRow {
                    key: "remote:github.com/acme/web".into(),
                    label: "web · 3 rules on".into(),
                    running: true,
                }],
                more: Some(Item {
                    label: "3 more…".into(),
                    enabled: true
                }),
            })
        );
        let unread = FULL.replacen(
            r#""attention":true"#,
            r#""attention":true,"repositories":null"#,
            1,
        );
        assert_eq!(
            serde_json::from_str::<MenuState>(&unread)
                .unwrap()
                .repositories,
            None
        );
    }

    #[test]
    fn reads_a_state_from_a_daemon_that_sends_no_repositories() {
        assert_eq!(
            serde_json::from_str::<MenuState>(FULL)
                .unwrap()
                .repositories,
            None
        );
    }

    #[test]
    fn ignores_unknown_fields() {
        let line = FULL.replacen('{', r#"{"x":1,"#, 1);
        assert!(serde_json::from_str::<MenuState>(&line).is_ok());
    }

    #[test]
    fn encodes_actions() {
        assert_eq!(encode_action(&Action::Open), "{\"action\":\"open\"}\n");
        assert_eq!(
            encode_action(&Action::Settings),
            "{\"action\":\"settings\"}\n"
        );
        assert_eq!(encode_action(&Action::Docs), "{\"action\":\"docs\"}\n");
        assert_eq!(
            encode_action(&Action::Repositories),
            "{\"action\":\"repositories\"}\n"
        );
        assert_eq!(
            encode_action(&Action::Restart),
            "{\"action\":\"restart\"}\n"
        );
        assert_eq!(encode_action(&Action::Update), "{\"action\":\"update\"}\n");
        assert_eq!(encode_action(&Action::Stop), "{\"action\":\"stop\"}\n");
    }

    #[test]
    fn encodes_a_repository_click_with_its_key_as_a_json_string() {
        let line = encode_action(&Action::Repository("local:/Users/a \"b\"\\c".into()));
        assert_eq!(
            line,
            "{\"action\":\"repository\",\"repository\":\"local:/Users/a \\\"b\\\"\\\\c\"}\n"
        );
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["repository"], "local:/Users/a \"b\"\\c");
    }

    #[test]
    fn read_states_closed_on_eof() {
        let (seen, notes) = collect(format!("{FULL}\n{FULL}\n").as_bytes());
        assert_eq!(notes, "");
        assert_eq!(seen.len(), 3);
        assert!(matches!(seen[0], Incoming::State(_)));
        assert!(matches!(seen[1], Incoming::State(_)));
        assert_eq!(seen[2], Incoming::Closed);
    }

    #[test]
    fn read_states_closed_on_read_error() {
        struct Failing;
        impl std::io::Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("pipe broke"))
            }
        }
        let (seen, notes) = collect(std::io::BufReader::new(Failing));
        assert_eq!(seen, [Incoming::Closed]);
        assert_eq!(
            notes,
            "rerail-tray: stopped reading menu states: pipe broke\n"
        );
    }

    #[test]
    fn read_states_skips_a_line_that_is_not_utf8() {
        let mut input = b"\xff\xfe\n".to_vec();
        input.extend_from_slice(format!("{FULL}\r\n{FULL}").as_bytes());
        let (seen, notes) = collect(input.as_slice());
        assert_eq!(notes.lines().count(), 1, "{notes:?}");
        assert!(
            notes.starts_with("rerail-tray: skipped a menu state that is not UTF-8: "),
            "{notes:?}"
        );
        // The line after it, and a last line with no newline, are still read.
        assert_eq!(seen.len(), 3);
        assert!(matches!(seen[0], Incoming::State(_)));
        assert!(matches!(seen[1], Incoming::State(_)));
        assert_eq!(seen[2], Incoming::Closed);
    }

    #[test]
    fn read_states_skips_bad_lines() {
        let (seen, notes) = collect(format!("not json\n{FULL}\n").as_bytes());
        assert_eq!(notes.lines().count(), 1, "{notes:?}");
        assert!(
            notes.starts_with("rerail-tray: skipped a menu state that did not parse: "),
            "{notes:?}"
        );
        assert_eq!(seen.len(), 2);
        assert!(matches!(seen[0], Incoming::State(_)));
        assert_eq!(seen[1], Incoming::Closed);
    }
}
