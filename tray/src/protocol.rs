//! The daemon talks to the tray in JSON lines: menu states arrive on stdin, clicks go back on stdout.

use std::io::{BufRead, Write};

use serde::Deserialize;

/// What the menu shows. The daemon sends one at start and another whenever anything in it changes.
#[derive(Deserialize, Clone, PartialEq, Debug)]
pub struct MenuState {
    pub healthy: bool,
    pub status: String,
    pub version: String,
    pub dashboard: String,
    pub home: Option<String>,
    pub update: Option<UpdateItem>,
    pub attention: bool,
}

#[derive(Deserialize, Clone, PartialEq, Debug)]
pub struct UpdateItem {
    pub label: String,
    pub enabled: bool,
}

/// A menu click the daemon acts on.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    Open,
    Update,
    Stop,
}

/// One JSON line, newline included, ready to write to stdout.
pub fn encode_action(action: Action) -> String {
    let name = match action {
        Action::Open => "open",
        Action::Update => "update",
        Action::Stop => "stop",
    };
    format!("{{\"action\":\"{name}\"}}\n")
}

/// Tells the daemon about a click. A failure is noted on stderr: the daemon is gone, or the click is lost.
pub fn write_action(action: Action) {
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

/// Sends every valid line as a state, skips an invalid one with a note on `diagnostics`, and sends `Closed` once at
/// end of input or on a read error, which also gets a note. The tray passes stderr, which lands in `daemon.log`.
pub fn read_states(
    input: impl BufRead,
    mut diagnostics: impl Write,
    mut send: impl FnMut(Incoming),
) {
    for line in input.lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                // A note that cannot be written has nowhere else to go.
                let _ = writeln!(
                    diagnostics,
                    "rerail-tray: stopped reading menu states: {error}"
                );
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<MenuState>(&line) {
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

    const FULL: &str = r#"{"healthy":true,"status":"Rerail is running","version":"Version 0.2.2","dashboard":"Dashboard at 127.0.0.1:4737","home":null,"update":{"label":"Update to 0.2.3","enabled":true},"attention":true}"#;

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
                dashboard: "Dashboard at 127.0.0.1:4737".into(),
                home: None,
                update: Some(UpdateItem {
                    label: "Update to 0.2.3".into(),
                    enabled: true
                }),
                attention: true,
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
    fn ignores_unknown_fields() {
        let line = FULL.replacen('{', r#"{"x":1,"#, 1);
        assert!(serde_json::from_str::<MenuState>(&line).is_ok());
    }

    #[test]
    fn encodes_actions() {
        assert_eq!(encode_action(Action::Open), "{\"action\":\"open\"}\n");
        assert_eq!(encode_action(Action::Update), "{\"action\":\"update\"}\n");
        assert_eq!(encode_action(Action::Stop), "{\"action\":\"stop\"}\n");
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
