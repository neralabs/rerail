//! The daemon talks to the tray in JSON lines: menu states arrive on stdin, clicks go back on stdout.

use std::io::BufRead;

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

#[derive(PartialEq, Debug)]
pub enum Incoming {
    State(MenuState),
    /// Stdin ended or failed: the daemon is gone, so the tray exits.
    Closed,
}

/// Sends every valid line as a state, skips an invalid one with a note on stderr (which lands in `daemon.log`),
/// and sends `Closed` once at end of input or on a read error.
pub fn read_states(input: impl BufRead, mut send: impl FnMut(Incoming)) {
    for line in input.lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("rerail-tray: stopped reading menu states: {error}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<MenuState>(&line) {
            Ok(state) => send(Incoming::State(state)),
            Err(error) => {
                eprintln!("rerail-tray: skipped a menu state that did not parse: {error}")
            }
        }
    }
    send(Incoming::Closed);
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{"healthy":true,"status":"Rerail is running","version":"Version 0.2.2","dashboard":"Dashboard at 127.0.0.1:4737","home":null,"update":{"label":"Update to 0.2.3","enabled":true},"attention":true}"#;

    fn collect(input: &str) -> Vec<Incoming> {
        let mut seen = Vec::new();
        read_states(input.as_bytes(), |incoming| seen.push(incoming));
        seen
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
        let seen = collect(&format!("{FULL}\n{FULL}\n"));
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
        let mut seen = Vec::new();
        read_states(std::io::BufReader::new(Failing), |incoming| {
            seen.push(incoming)
        });
        assert_eq!(seen, [Incoming::Closed]);
    }

    #[test]
    fn read_states_skips_bad_lines() {
        let seen = collect(&format!("not json\n{FULL}\n"));
        assert_eq!(seen.len(), 2);
        assert!(matches!(seen[0], Incoming::State(_)));
        assert_eq!(seen[1], Incoming::Closed);
    }
}
